"""The function name is a number: the function survives, the name is an error."""

import os


def 1fetch(url):
    with open(url) as handle:
        return handle.read()


def ok():
    return os.getcwd()
