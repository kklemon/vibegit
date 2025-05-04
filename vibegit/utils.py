import re
import warnings
from importlib.metadata import version


def get_version():
    return version("vibegit")


def compare_versions(version1, version2):
    def normalize(v):
        return [int(x) for x in re.sub(r"(\.0+)*$", "", v).split(".")]

    return normalize(version1) > normalize(version2)


def check_for_update():
    try:
        import requests

        version = get_version()

        try:
            response = requests.get("https://pypi.org/pypi/vibegit/json", timeout=1)
            latest_version = response.json()["info"]["version"]

            if compare_versions(latest_version, version):
                warnings.warn(
                    f'You are using vibegit version {version}, however version {latest_version} is available. You should consider upgrading via the "pip install --upgrade vibegit" command.'
                )
        except (
            requests.exceptions.RequestException,
            requests.exceptions.ConnectionError,
            requests.exceptions.HTTPError,
            requests.exceptions.SSLError,
            requests.exceptions.Timeout,
        ):
            # when pypi servers or le internet is down
            pass
    except ModuleNotFoundError:
        pass
