"""Minimal prompt types for noninteractive read tests."""


class Prompts:
    def __init__(self, shell):
        self.shell = shell


class Token:
    Prompt = "Prompt"
    PromptNum = "PromptNum"
    OutPrompt = "OutPrompt"
    OutPromptNum = "OutPromptNum"
