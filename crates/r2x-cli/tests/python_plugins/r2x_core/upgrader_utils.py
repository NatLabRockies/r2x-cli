"""Stub upgrader helpers for integration tests."""

from __future__ import annotations


class Result:
    """Simple result helper mirroring the expected API."""

    def __init__(self, value=None, error=None):
        self._value = value
        self._error = error

    def is_err(self) -> bool:
        return self._error is not None

    def unwrap(self):
        if self._error is not None:
            raise RuntimeError(self._error)
        return self._value

    def unwrap_err(self):
        if self._error is None:
            raise RuntimeError("no error present")
        return self._error


def run_upgrade_step(data, *, step, upgrader_context=None):
    """Return an ok result with the provided data."""
    return Result(value=data)
