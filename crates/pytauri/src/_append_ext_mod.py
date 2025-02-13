import sys
from types import ModuleType
from importlib.metadata import entry_points

__all__ = ["append_ext_mod"]


def append_ext_mod(ext_mod: ModuleType) -> None:
    if sys.version_info >= (3, 10):
        # To avoid deprecation warnings
        eps = entry_points(group="pytauri", name="ext_mod")
    else:
        # TODO: how to specify the name?
        eps = entry_points()["pytauri"]

    ext_mod_path = tuple(eps)[0].value

    sys.modules[ext_mod_path] = ext_mod
