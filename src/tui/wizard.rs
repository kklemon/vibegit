//! First-run configuration wizard: pick a model, store the API key.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::DefaultTerminal;
use tui_textarea::TextArea;

use super::widgets::{centered_fixed, hint_line};
use crate::config::{config_path, Config, ModelConfig};
use crate::llm::{
    api_key_names, check_model, provider_adapter, split_model_name, ModelCheck, SUPPORTED_PROVIDERS,
};
use crate::openrouter::{fetch_models, filter_models, looks_like_model, OpenRouterModel};

// Lineup checked against the provider documentation on 2026-10-27.
const PRESETS: &[(&str, &str)] = &[
    ("Gemini 3.8 Flash (recommended)", "google:gemini-3.8-flash"),
    (
        "Gemini 3.5 Flash-Lite (fastest, lowest cost)",
        "google:gemini-3.5-flash-lite",
    ),
    ("Gemini 4 Argon (flagship)", "google:gemini-4-argon"),
    ("GPT-6.1 Terra (fast and efficient)", "openai:gpt-6.1-terra"),
    ("GPT-6.1 Sol (high quality)", "openai:gpt-6.1-sol"),
    ("GPT-6 Luna (high volume, lowest cost)", "openai:gpt-6-luna"),
    (
        "Claude Sonnet 5.5 (balanced)",
        "anthropic:claude-sonnet-5-5",
    ),
    (
        "Claude Haiku 5.5 (fast, low cost)",
        "anthropic:claude-haiku-5-5",
    ),
    ("Claude Opus 5.5 (flagship)", "anthropic:claude-opus-5-5"),
    ("Grok 4.7 (SpaceXSI, formerly xAI)", "grok:grok-4.7"),
    ("Grok 4.3 (lower cost, 1M context)", "grok:grok-4.3"),
    ("OpenRouter (any vendor's model with one key)", OPENROUTER),
    ("Custom OpenAI-compatible endpoint", CUSTOM),
    (
        "Other provider:model (ollama, groq, deepseek, openrouter, ...)",
        OTHER,
    ),
];
const CUSTOM: &str = "<custom>";
const OTHER: &str = "<other>";
const OPENROUTER: &str = "<openrouter>";
const TICK: Duration = Duration::from_millis(80);
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

enum Step {
    ChooseModel {
        index: usize,
    },
    CustomEndpoint {
        field: usize,
        inputs: Vec<TextArea<'static>>,
    },
    OtherModel {
        input: TextArea<'static>,
        error: Option<String>,
    },
    OpenRouter {
        input: TextArea<'static>,
        catalog: Catalog,
        selected: usize,
        error: Option<String>,
    },
    ApiKey {
        env: String,
        found_in_env: bool,
        input: TextArea<'static>,
    },
    /// Everything is entered; `finish` saves the file and starts the test request.
    Ready,
    Testing {
        rx: mpsc::Receiver<Result<ModelCheck, String>>,
        started: Instant,
    },
    Done {
        check: Result<ModelCheck, String>,
    },
}

/// The OpenRouter catalog while it downloads in the background, and afterwards.
enum Catalog {
    Loading {
        rx: mpsc::Receiver<Result<Vec<OpenRouterModel>, String>>,
        started: Instant,
    },
    Ready(Vec<OpenRouterModel>),
    Failed(String),
}

struct Wizard {
    config: Config,
    step: Step,
    message: Option<String>,
    /// The configuration as last written to disk, once `finish` ran.
    saved: Option<Config>,
}

fn single_line(initial: &str, placeholder: &str, masked: bool) -> TextArea<'static> {
    let mut input = TextArea::new(vec![initial.to_string()]);
    input.set_cursor_line_style(Style::default());
    input.set_placeholder_text(placeholder);
    if masked {
        input.set_mask_char('•');
    }
    input.move_cursor(tui_textarea::CursorMove::End);
    input
}

fn value(input: &TextArea<'_>) -> String {
    input.lines().join("").trim().to_string()
}

pub fn run(terminal: &mut DefaultTerminal, config: Config) -> Result<Option<Config>> {
    let mut wizard = Wizard {
        config,
        step: Step::ChooseModel { index: 0 },
        message: None,
        saved: None,
    };
    loop {
        terminal.draw(|frame| render(frame, &mut wizard))?;
        wizard.tick();
        if !event::poll(TICK)? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(wizard.saved.take());
            }
            match wizard.on_key(key)? {
                Flow::Continue => {}
                // A file written before the user backed out stays valid.
                Flow::Cancel => return Ok(wizard.saved.take()),
                Flow::Finished => return Ok(wizard.saved.take().or(Some(wizard.config))),
            }
        }
    }
}

enum Flow {
    Continue,
    Cancel,
    Finished,
}

impl Wizard {
    fn on_key(&mut self, key: KeyEvent) -> Result<Flow> {
        let step = std::mem::replace(&mut self.step, Step::Ready);
        match step {
            Step::ChooseModel { mut index } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(Flow::Cancel),
                KeyCode::Up | KeyCode::Char('k') => {
                    index = (index + PRESETS.len() - 1) % PRESETS.len();
                    self.step = Step::ChooseModel { index };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    index = (index + 1) % PRESETS.len();
                    self.step = Step::ChooseModel { index };
                }
                KeyCode::Enter => self.choose(index),
                _ => self.step = Step::ChooseModel { index },
            },
            Step::CustomEndpoint {
                mut field,
                mut inputs,
            } => match key.code {
                KeyCode::Esc => return Ok(Flow::Cancel),
                KeyCode::Tab | KeyCode::Down => {
                    field = (field + 1) % inputs.len();
                    self.step = Step::CustomEndpoint { field, inputs };
                }
                KeyCode::BackTab | KeyCode::Up => {
                    field = (field + inputs.len() - 1) % inputs.len();
                    self.step = Step::CustomEndpoint { field, inputs };
                }
                KeyCode::Enter => {
                    if field + 1 < inputs.len() {
                        self.step = Step::CustomEndpoint {
                            field: field + 1,
                            inputs,
                        };
                    } else {
                        let base_url = value(&inputs[0]);
                        let model = value(&inputs[1]);
                        let api_key = value(&inputs[2]);
                        if base_url.is_empty() || model.is_empty() {
                            self.message = Some("Base URL and model name are required.".into());
                            self.step = Step::CustomEndpoint { field: 0, inputs };
                        } else {
                            self.config.model = ModelConfig {
                                name: model,
                                base_url: Some(base_url),
                                api_key: if api_key.is_empty() {
                                    None
                                } else {
                                    Some(api_key)
                                },
                                model_provider: Some("openai".into()),
                                temperature: self.config.model.temperature,
                            };
                            return self.finish();
                        }
                    }
                }
                _ => {
                    inputs[field].input(key);
                    self.step = Step::CustomEndpoint { field, inputs };
                }
            },
            Step::OtherModel { mut input, .. } => match key.code {
                KeyCode::Esc => return Ok(Flow::Cancel),
                KeyCode::Enter => {
                    let name = value(&input);
                    let (provider, _) = split_model_name(&name, None);
                    match provider.and_then(|p| provider_adapter(&p)) {
                        Some((adapter, _)) => {
                            self.config.model = ModelConfig {
                                name,
                                temperature: self.config.model.temperature,
                                ..ModelConfig::default()
                            };
                            self.start_api_key_step(api_key_names(adapter));
                            if matches!(self.step, Step::Ready) {
                                return self.finish();
                            }
                        }
                        None => {
                            self.step = Step::OtherModel {
                                input,
                                error: Some(format!(
                                    "Use <provider>:<model> with one of: {}",
                                    SUPPORTED_PROVIDERS.join(", ")
                                )),
                            };
                        }
                    }
                }
                _ => {
                    input.input(key);
                    self.step = Step::OtherModel { input, error: None };
                }
            },
            Step::OpenRouter {
                mut input,
                catalog,
                mut selected,
                ..
            } => match key.code {
                KeyCode::Esc => return Ok(Flow::Cancel),
                KeyCode::Up | KeyCode::Down => {
                    let shown = match &catalog {
                        Catalog::Ready(models) => filter_models(models, &value(&input)).len(),
                        _ => 0,
                    };
                    if shown > 0 {
                        selected = if key.code == KeyCode::Up {
                            (selected + shown - 1) % shown
                        } else {
                            (selected + 1) % shown
                        };
                    }
                    self.step = Step::OpenRouter {
                        input,
                        catalog,
                        selected,
                        error: None,
                    };
                }
                KeyCode::Enter => {
                    let typed = value(&input);
                    let highlighted = match &catalog {
                        Catalog::Ready(models) => filter_models(models, &typed)
                            .get(selected)
                            .map(|m| m.id.clone()),
                        _ => None,
                    };
                    let chosen = highlighted.unwrap_or(typed);
                    if looks_like_model(&chosen) {
                        self.config.model = ModelConfig {
                            name: format!("openrouter:{chosen}"),
                            temperature: self.config.model.temperature,
                            ..ModelConfig::default()
                        };
                        self.start_api_key_step(api_key_names(
                            genai::adapter::AdapterKind::OpenRouter,
                        ));
                        if matches!(self.step, Step::Ready) {
                            return self.finish();
                        }
                    } else {
                        self.step = Step::OpenRouter {
                            input,
                            catalog,
                            selected,
                            error: Some(
                                "Enter a model as <vendor>/<model>, e.g. google/gemini-3.8-flash."
                                    .into(),
                            ),
                        };
                    }
                }
                _ => {
                    input.input(key);
                    self.step = Step::OpenRouter {
                        input,
                        catalog,
                        selected: 0,
                        error: None,
                    };
                }
            },
            Step::ApiKey {
                env,
                found_in_env,
                mut input,
            } => {
                if found_in_env {
                    match key.code {
                        KeyCode::Esc => return Ok(Flow::Cancel),
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            if let Ok(v) = std::env::var(&env) {
                                self.config.api_keys.insert(env.clone(), v);
                            }
                            return self.finish();
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            self.message =
                                Some(format!("{env} will be read from the environment."));
                            return self.finish();
                        }
                        _ => {
                            self.step = Step::ApiKey {
                                env,
                                found_in_env,
                                input,
                            }
                        }
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => return Ok(Flow::Cancel),
                        KeyCode::Enter => {
                            let key_value = value(&input);
                            if key_value.is_empty() {
                                self.message = Some("The API key cannot be empty.".into());
                                self.step = Step::ApiKey {
                                    env,
                                    found_in_env,
                                    input,
                                };
                            } else {
                                self.config.api_keys.insert(env, key_value);
                                return self.finish();
                            }
                        }
                        _ => {
                            input.input(key);
                            self.step = Step::ApiKey {
                                env,
                                found_in_env,
                                input,
                            };
                        }
                    }
                }
            }
            Step::Ready => self.step = Step::Ready,
            Step::Testing { rx, started } => match key.code {
                // The file is already written, so the test can be skipped.
                KeyCode::Esc | KeyCode::Char('q') => return Ok(Flow::Finished),
                _ => self.step = Step::Testing { rx, started },
            },
            Step::Done { check } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => return Ok(Flow::Finished),
                KeyCode::Char('r') if check.is_err() => self.start_check(),
                KeyCode::Char('m') if check.is_err() => {
                    self.message = None;
                    self.step = Step::ChooseModel { index: 0 };
                }
                _ => self.step = Step::Done { check },
            },
        }
        Ok(Flow::Continue)
    }

    fn choose(&mut self, index: usize) {
        let (_, model) = PRESETS[index];
        match model {
            CUSTOM => {
                let existing = &self.config.model;
                let is_custom = existing.base_url.is_some();
                let base_url = if is_custom {
                    existing.base_url.clone().unwrap_or_default()
                } else {
                    String::new()
                };
                let name = if is_custom {
                    existing.name.clone()
                } else {
                    String::new()
                };
                let key = if is_custom {
                    existing.api_key.clone().unwrap_or_default()
                } else {
                    String::new()
                };
                self.step = Step::CustomEndpoint {
                    field: 0,
                    inputs: vec![
                        single_line(&base_url, "https://api.example.com/v1", false),
                        single_line(&name, "model name", false),
                        single_line(&key, "API key (optional for local servers)", true),
                    ],
                };
            }
            OPENROUTER => {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(fetch_models());
                });
                self.step = Step::OpenRouter {
                    input: single_line("", "type to search, e.g. google/gemini-3.8-flash", false),
                    catalog: Catalog::Loading {
                        rx,
                        started: Instant::now(),
                    },
                    selected: 0,
                    error: None,
                };
            }
            OTHER => {
                self.step = Step::OtherModel {
                    input: single_line("", "ollama:llama3   groq:...   openrouter:...", false),
                    error: None,
                };
            }
            preset => {
                self.config.model = ModelConfig {
                    name: preset.to_string(),
                    temperature: self.config.model.temperature,
                    ..ModelConfig::default()
                };
                let (provider, _) = split_model_name(preset, None);
                let adapter = provider.and_then(|p| provider_adapter(&p)).map(|(a, _)| a);
                match adapter {
                    Some(a) => self.start_api_key_step(api_key_names(a)),
                    None => self.step = Step::Ready,
                }
                if matches!(self.step, Step::Ready) {
                    let _ = self.finish();
                }
            }
        }
    }

    /// Moves to the API key step, or straight to `Done` when a key is already configured.
    fn start_api_key_step(&mut self, names: &[&str]) {
        let Some(primary) = names.first() else {
            self.step = Step::Ready;
            return;
        };
        if names.iter().any(|n| {
            self.config
                .api_keys
                .get(*n)
                .is_some_and(|v| !v.trim().is_empty())
        }) {
            self.message = Some(format!("{primary} is already stored in the configuration."));
            self.step = Step::Ready;
            return;
        }
        let found = names
            .iter()
            .find(|n| std::env::var(n).is_ok_and(|v| !v.trim().is_empty()));
        self.step = Step::ApiKey {
            env: found
                .map(|n| n.to_string())
                .unwrap_or_else(|| primary.to_string()),
            found_in_env: found.is_some(),
            input: single_line("", "paste the key and press Enter", true),
        };
    }

    /// Writes the configuration and sends the test request in the background.
    fn finish(&mut self) -> Result<Flow> {
        self.config.save()?;
        self.saved = Some(self.config.clone());
        self.start_check();
        Ok(Flow::Continue)
    }

    fn start_check(&mut self) {
        let (tx, rx) = mpsc::channel();
        let config = self.config.clone();
        std::thread::spawn(move || {
            let result = check_model(&config).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.step = Step::Testing {
            rx,
            started: Instant::now(),
        };
    }

    /// Picks up background results: the OpenRouter catalog and the test request.
    fn tick(&mut self) {
        match &mut self.step {
            Step::Testing { rx, .. } => {
                if let Ok(check) = rx.try_recv() {
                    self.step = Step::Done { check };
                }
            }
            Step::OpenRouter { catalog, .. } => {
                if let Catalog::Loading { rx, .. } = catalog {
                    if let Ok(result) = rx.try_recv() {
                        *catalog = match result {
                            Ok(models) => Catalog::Ready(models),
                            Err(err) => Catalog::Failed(err),
                        };
                    }
                }
            }
            _ => {}
        }
    }
}

fn saved_lines(config: &Config) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Configuration saved",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  model: {}", config.model.name)),
        Line::from(format!("  file:  {}", config_path().display())),
    ]
}

/// Collapses an error to one short line so an HTTP body cannot push the hints off screen.
fn brief(error: &str) -> String {
    let flat = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(240).collect();
    if flat.chars().count() > 240 {
        out.push('…');
    }
    out
}

fn render(frame: &mut Frame, wizard: &mut Wizard) {
    let area = frame.area();
    let rect = centered_fixed(84.min(area.width), 24.min(area.height), area);
    let block = Block::bordered()
        .title(" VibeGit setup ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let heading = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let hints: Vec<(&str, &str)>;

    match &mut wizard.step {
        Step::ChooseModel { index } => {
            let items: Vec<ListItem> = PRESETS
                .iter()
                .map(|(label, model)| {
                    let detail = match *model {
                        CUSTOM | OTHER | OPENROUTER => String::new(),
                        m => format!("  {m}"),
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw(label.to_string()),
                        Span::styled(detail, Style::default().fg(Color::DarkGray)),
                    ]))
                })
                .collect();
            let [title_area, list_area] =
                Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(body);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        "Choose the model that writes your commits",
                        heading,
                    )),
                    Line::from(Span::styled(
                        format!("current: {}", wizard.config.model.name),
                        Style::default().fg(Color::DarkGray),
                    )),
                ]),
                title_area,
            );
            let mut state = ListState::default().with_selected(Some(*index));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                    .highlight_symbol("▶ "),
                list_area,
                &mut state,
            );
            hints = vec![("↑↓", "choose"), ("Enter", "select"), ("Esc", "cancel")];
        }
        Step::CustomEndpoint { field, inputs } => {
            let [title_area, f1, f2, f3, msg] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .areas(body);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "OpenAI-compatible endpoint",
                    heading,
                ))),
                title_area,
            );
            let titles = [" Base URL ", " Model name ", " API key "];
            for (i, (input, rect)) in inputs.iter_mut().zip([f1, f2, f3]).enumerate() {
                let color = if i == *field {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                input.set_block(
                    Block::bordered()
                        .title(titles[i])
                        .border_style(Style::default().fg(color)),
                );
                frame.render_widget(&*input, rect);
            }
            if let Some(m) = &wizard.message {
                frame.render_widget(
                    Paragraph::new(Span::styled(m.clone(), Style::default().fg(Color::Red))),
                    msg,
                );
            }
            hints = vec![
                ("Tab", "next field"),
                ("Enter", "next / save"),
                ("Esc", "cancel"),
            ];
        }
        Step::OtherModel { input, error } => {
            let [title_area, f1, msg] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .areas(body);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "Enter the model as <provider>:<model>",
                    heading,
                ))),
                title_area,
            );
            input.set_block(
                Block::bordered()
                    .title(" Model ")
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            frame.render_widget(&*input, f1);
            let mut lines = vec![Line::from(Span::styled(
                format!("providers: {}", SUPPORTED_PROVIDERS.join(", ")),
                Style::default().fg(Color::DarkGray),
            ))];
            if let Some(e) = error {
                lines.push(Line::from(Span::styled(
                    e.clone(),
                    Style::default().fg(Color::Red),
                )));
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), msg);
            hints = vec![("Enter", "continue"), ("Esc", "cancel")];
        }
        Step::OpenRouter {
            input,
            catalog,
            selected,
            error,
        } => {
            let [title_area, f1, status_area, list_area] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .areas(body);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled("OpenRouter model", heading)),
                    Line::from(Span::styled(
                        "one key for every vendor's models, billed by openrouter.ai",
                        Style::default().fg(Color::DarkGray),
                    )),
                ]),
                title_area,
            );
            input.set_block(
                Block::bordered()
                    .title(" Model ")
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            frame.render_widget(&*input, f1);
            let dim = Style::default().fg(Color::DarkGray);
            let filter = value(input);
            let (status, shown): (Line, Vec<&OpenRouterModel>) = match catalog {
                Catalog::Loading { started, .. } => {
                    let spinner =
                        SPINNER[(started.elapsed().as_millis() / 100) as usize % SPINNER.len()];
                    (
                        Line::from(Span::styled(
                            format!("{spinner} Loading the model list from openrouter.ai..."),
                            dim,
                        )),
                        Vec::new(),
                    )
                }
                Catalog::Ready(models) => {
                    let shown = filter_models(models, &filter);
                    (
                        Line::from(Span::styled(
                            format!("{} of {} models", shown.len(), models.len()),
                            dim,
                        )),
                        shown,
                    )
                }
                Catalog::Failed(err) => (
                    Line::from(Span::styled(
                        format!(
                            "Could not load the model list ({}); enter the full model name.",
                            brief(err)
                        ),
                        Style::default().fg(Color::Yellow),
                    )),
                    Vec::new(),
                ),
            };
            let status = match error {
                Some(e) => Line::from(Span::styled(e.clone(), Style::default().fg(Color::Red))),
                None => status,
            };
            frame.render_widget(Paragraph::new(status), status_area);
            let items: Vec<ListItem> = shown
                .iter()
                .map(|m| {
                    ListItem::new(Line::from(vec![
                        Span::raw(m.id.clone()),
                        Span::styled(format!("  {}", m.summary()), dim),
                    ]))
                })
                .collect();
            let highlighted = (!items.is_empty()).then(|| (*selected).min(items.len() - 1));
            let mut state = ListState::default().with_selected(highlighted);
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                    .highlight_symbol("▶ "),
                list_area,
                &mut state,
            );
            hints = vec![
                ("type", "search"),
                ("↑↓", "choose"),
                ("Enter", "select"),
                ("Esc", "cancel"),
            ];
        }
        Step::ApiKey {
            env,
            found_in_env,
            input,
        } => {
            let [title_area, f1, msg] = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .areas(body);
            if *found_in_env {
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(Span::styled(
                            format!("{env} found in your environment"),
                            heading,
                        )),
                        Line::from(""),
                        Line::from(format!(
                            "Save it to the VibeGit configuration? [y/n]  (n keeps reading ${env})"
                        )),
                    ]),
                    title_area,
                );
                hints = vec![("y", "save"), ("n", "use environment"), ("Esc", "cancel")];
            } else {
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(Span::styled(format!("Enter your {env}"), heading)),
                        Line::from(Span::styled(
                            format!("model: {}", wizard.config.model.name),
                            Style::default().fg(Color::DarkGray),
                        )),
                    ]),
                    title_area,
                );
                input.set_block(
                    Block::bordered()
                        .title(format!(" {env} "))
                        .border_style(Style::default().fg(Color::Cyan)),
                );
                frame.render_widget(&*input, f1);
                if let Some(m) = &wizard.message {
                    frame.render_widget(
                        Paragraph::new(Span::styled(m.clone(), Style::default().fg(Color::Red))),
                        msg,
                    );
                }
                hints = vec![("Enter", "save"), ("Esc", "cancel")];
            }
        }
        Step::Ready => hints = Vec::new(),
        Step::Testing { started, .. } => {
            let elapsed = started.elapsed();
            let spinner = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
            let mut lines = saved_lines(&wizard.config);
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "  {spinner} Sending a test request to {}... {}s",
                wizard.config.model.name,
                elapsed.as_secs()
            )));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
            hints = vec![("Esc", "skip the test")];
        }
        Step::Done { check } => {
            let mut lines = saved_lines(&wizard.config);
            if let Some(m) = &wizard.message {
                lines.push(Line::from(""));
                lines.push(Line::from(format!("  {m}")));
            }
            lines.push(Line::from(""));
            match check {
                Ok(c) => {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "  ✓ {} answered the test request in {:.1}s.",
                            c.label,
                            c.elapsed.as_secs_f64()
                        ),
                        Style::default().fg(Color::Green),
                    )));
                    hints = vec![("Enter", "finish")];
                }
                Err(e) => {
                    lines.push(Line::from(Span::styled(
                        "  ✗ The test request failed:",
                        Style::default().fg(Color::Red),
                    )));
                    lines.push(Line::from(format!("    {}", brief(e))));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "  Check the API key and the model name; the settings are saved either way.",
                        Style::default().fg(Color::DarkGray),
                    )));
                    hints = vec![
                        ("Enter", "keep anyway"),
                        ("r", "retry"),
                        ("m", "change model"),
                    ];
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Change settings later with `vibegit config set <path> <value>`.",
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        }
    }
    frame.render_widget(Paragraph::new(hint_line(&hints)), footer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_names_a_supported_provider_exactly_once() {
        let mut seen = std::collections::HashSet::new();
        for (label, model) in PRESETS {
            assert!(seen.insert(*model), "duplicate preset {model}");
            if *model == CUSTOM || *model == OTHER || *model == OPENROUTER {
                continue;
            }
            let (provider, name) = split_model_name(model, None);
            let provider = provider.unwrap_or_else(|| panic!("{label}: missing provider"));
            assert!(
                provider_adapter(&provider).is_some(),
                "{label}: unsupported provider {provider}"
            );
            assert!(
                !name.is_empty() && !name.contains(' '),
                "{label}: bad model {name}"
            );
        }
        assert_eq!(PRESETS[0].1, crate::config::DEFAULT_MODEL);
    }
}
