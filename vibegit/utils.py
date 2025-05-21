import re
from importlib.metadata import version


def get_version():
    return version("vibegit")


def compare_versions(version1: str, version2: str) -> bool:
    def normalize(v: str) -> list[int]:
        return [int(x) for x in re.sub(r"(\.0+)*$", "", v).split(".")]

    return normalize(version1) > normalize(version2)
