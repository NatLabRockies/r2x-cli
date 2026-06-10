"""Minimal InteractiveShellEmbed stub for noninteractive read tests."""


class InteractiveShellEmbed:
    def __init__(self, *args, **kwargs):
        self.execution_count = 1
        self.user_ns = {}
        self.prompts = None

    def register_magic_function(self, *args, **kwargs):
        return None

    def __call__(self, *args, **kwargs):
        return None
