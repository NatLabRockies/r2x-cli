"""Minimal traitlets.config stub for noninteractive read tests."""


class _Section:
    pass


class Config:
    def __init__(self):
        self.TerminalInteractiveShell = _Section()
        self.HistoryManager = _Section()
