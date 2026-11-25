"""Shared rules for plain UI messages. Uses only the Python standard library."""
import json
import re
from collections import Counter
from pathlib import Path


# A shortcut such as <Enter> is valid UI text. HTML/XML document elements,
# closing tags and attributes belong in the renderer.
MARKUP = re.compile(
    r'<!doctype\b|<!--|<\?xml\b|</[a-z][\w:.-]*\s*>|<[a-z][\w:.-]*\s+[^<>]*='
    r'|<(?:html|head|meta|body|style|script|title|link|nav|div|span|p|h[1-6]|a|img|svg|path|br|b|em|strong|table)\s*/?>',
    re.I,
)
FIELD = re.compile(r'\{\{|\}\}|\{[^{}]*\}|[{}]')


def fields(text):
    tokens = FIELD.findall(text)
    if '{' in tokens or '}' in tokens:
        raise ValueError('unmatched format brace')
    return Counter(tokens)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate JSON key {key!r}')
        result[key] = value
    return result


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=unique_object)


def validate(messages):
    if not isinstance(messages, dict):
        raise ValueError('expected a JSON object')
    for key, value in messages.items():
        if not key.strip() or not isinstance(value, str) or not value.strip():
            raise ValueError(f'empty or invalid message for {key!r}')
        if MARKUP.search(key) or MARKUP.search(value):
            raise ValueError(f'export markup in UI message {key[:80]!r}')
        if fields(key) != fields(value):
            raise ValueError(f'format fields changed in {key!r}')


def check_bundled(root):
    source = (root / 'crates/map_app/src/i18n.rs').read_text(encoding='utf-8')
    languages = set(re.findall(r'include_language!\("([^"]+)"\)', source))
    paths = sorted((root / 'assets/i18n').glob('*.json'))
    if not languages or {p.stem for p in paths} != languages:
        raise ValueError('catalogue files differ from the languages embedded by the desktop app')
    for path in paths:
        try:
            validate(read_json(path))
        except ValueError as error:
            raise ValueError(f'{path.name}: {error}') from error
    return len(paths)


if __name__ == '__main__':
    print(f'Validated {check_bundled(Path(__file__).resolve().parents[1])} UI catalogues')
