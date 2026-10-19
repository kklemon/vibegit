//! First-run configuration wizard: pick a model, store the API key.

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::DefaultTerminal;
use tui_textarea::TextArea;

use super::widgets::{centered_fixed, hint_line};
use crate::config::{config_path, Config, ModelConfig};
use crate::llm::{api_key_names, provider_adapter, split_model_name, SUPPORTED_PROVIDERS};

const PRESETS: &[(&str, &str)] = &[
    ("Gemini 3.7 Flash (recommended)", "google:gemini-3.8-flash"),
    (
        "Gemini 3.5 Flash-Lite (fast and cost-efficient)",
        "google:gemini-3.5-flash-lite",
    ),
    ("Gemini 3.1 Pro (preview)", "google:gemini-3.1-pro-preview"),
    ("GPT-5.6 Terra (balanced)", "openai:gpt-5.6-terra"),
    ("GPT-5.6 Sol (highest quality)", "openai:gpt-5.6-sol"),
    ("GPT-5.6 Luna (high volume)", "openai:gpt-5.6-luna"),
    ("Grok Code Fast", "grok:grok-code-fast-1"),
    ("Custom OpenAI-compatible endpoint", CUSTOM),
    (
        "Other provider:model (anthropic, ollama, groq, deepseek, ...)",
        OTHER,
    ),
];
const CUSTOM: &str = "<custom>";
const OTHER: &str = "<other>";

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
    ApiKey {
        env: String,
        found_in_env: bool,
        input: TextArea<'static>,
    },
    Done,
}

struct Wizard {
    config: Config,
    step: Step,
    message: Option<String>,
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
    };
    loop {
        terminal.draw(|frame| render(frame, &mut wizard))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(None);
            }
            match wizard.on_key(key)? {
                Flow::Continue => {}
                Flow::Cancel => return Ok(None),
                Flow::Finished => return Ok(Some(wizard.config)),
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
        let step = std::mem::replace(&mut self.step, Step::Done);
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
                            if matches!(self.step, Step::Done) {
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
            Step::Done => {
                self.step = Step::Done;
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                    return Ok(Flow::Finished);
                }
            }
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
            OTHER => {
                self.step = Step::OtherModel {
                    input: single_line(
                        "",
                        "anthropic:claude-...   ollama:llama3   groq:...",
                        false,
                    ),
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
                    None => self.step = Step::Done,
                }
                if matches!(self.step, Step::Done) {
                    let _ = self.finish();
                }
            }
        }
    }

    /// Moves to the API key step, or straight to `Done` when a key is already configured.
    fn start_api_key_step(&mut self, names: &[&str]) {
        let Some(primary) = names.first() else {
            self.step = Step::Done;
            return;
        };
        if names.iter().any(|n| {
            self.config
                .api_keys
                .get(*n)
                .is_some_and(|v| !v.trim().is_empty())
        }) {
            self.message = Some(format!("{primary} is already stored in the configuration."));
            self.step = Step::Done;
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

    fn finish(&mut self) -> Result<Flow> {
        self.config.save()?;
        self.step = Step::Done;
        Ok(Flow::Continue)
    }
}

fn render(frame: &mut Frame, wizard: &mut Wizard) {
    let area = frame.area();
    let rect = centered_fixed(84.min(area.width), 20.min(area.height), area);
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
                        CUSTOM | OTHER => String::new(),
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
        Step::Done => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Configuration saved",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!("  model: {}", wizard.config.model.name)),
                Line::from(format!("  file:  {}", config_path().display())),
            ];
            if let Some(m) = &wizard.message {
                lines.push(Line::from(""));
                lines.push(Line::from(format!("  {m}")));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Change settings later with `vibegit config set <path> <value>`.",
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
            hints = vec![("Enter", "finish")];
        }
    }
    frame.render_widget(Paragraph::new(hint_line(&hints)), footer);
}
