"""Extract and wrap translatable Rust UI strings.

Use tree-sitter and tree-sitter-rust, then run:
    python tools/localize_ui.py --inventory target/atelier-qa/ui-keys.json
    python tools/localize_ui.py --wrap

Only calls known to display labels are rewritten. User data, identifiers,
format filenames, and untranslated input to protocols are left alone.
"""
import argparse
import json
import re
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust
from i18n_catalog import MARKUP


HAN = re.compile(r'[\u3400-\u9fff]')
SIMPLE = {
    'label', 'weak', 'strong', 'heading', 'monospace', 'button', 'small_button',
    'link', 'menu_button', 'on_hover_text', 'on_disabled_hover_text',
    'hint_text', 'selected_text', 'text', 'title', 'prefix', 'suffix',
    'with_title', 'add_filter', 'set_title', 'set_status', 'set_file_name',
}
LABEL_INDEX = {
    'checkbox': 1, 'radio_value': 2, 'selectable_value': 2, 'selectable_label': 1,
    'colored_label': 1, 'hyperlink_to': 0, 'book_result': 1,
}
TEXT_CONSTRUCTORS = {
    'Button', 'Checkbox', 'CollapsingHeader', 'Label', 'RichText', 'Window',
    'Link', 'SelectableLabel', 'TextFormat',
}
TEXT_CONTAINERS = {'if_expression', 'else_clause', 'block', 'match_expression',
                   'match_block', 'match_arm', 'reference_expression', 'parenthesized_expression'}
FILE_SUFFIX = re.compile(r'\.(atlas|json|pdf|zip|png|svg|csv|docx|md|txt|jpg|gif|html|shp|obj|glb|log)(?:$|[^\w])', re.I)
LIBRARY_FOLDERS = ('map_app', 'map_core', 'map_io')
PARSER = Parser(Language(tree_sitter_rust.language()))
ROOT = Path(__file__).resolve().parent.parent


def walk(root, skip_name_pools=False):
    todo = [root]
    while todo:
        node = todo.pop()
        if skip_name_pools and node.type == 'const_item':
            declared = node.child_by_field_name('type')
            if declared and re.sub(rb'\s+', b'', declared.text) == b'&[&str]':
                continue
        yield node
        if node.type not in ('string_literal', 'raw_string_literal', 'line_comment', 'block_comment'):
            todo.extend(reversed(node.named_children))


def value(node):
    raw = node.text.decode('utf-8')
    if raw.startswith('r'):
        return raw.split('"', 1)[1].rsplit('"', 1)[0]
    try:
        return json.loads(raw)
    except ValueError:
        return None


def display_argument(expression):
    node = expression
    while node.parent is not None and node.parent.type in TEXT_CONTAINERS:
        node = node.parent
    args = node.parent
    if args is None or args.type != 'arguments':
        return False
    callee = args.parent.child_by_field_name('function')
    if callee is None:
        return False
    index = args.named_children.index(node)
    if callee.type == 'field_expression':
        field = callee.child_by_field_name('field').text.decode()
        if field == 'text' and len(args.named_children) >= 5:
            return index == 2
        if field in LABEL_INDEX:
            return index == LABEL_INDEX[field]
        return field in SIMPLE and index == 0 and field != 'set_file_name'
    text = callee.text.decode('utf-8')
    names = text.split('::')
    return index == 0 and len(names) >= 2 and names[-1] == 'new' and names[-2] in TEXT_CONSTRUCTORS


def in_test(node):
    # Test modules use separate fixture names, many written in Chinese.
    cur = node
    while cur.parent is not None:
        if cur.type == 'mod_item' and cur.child_by_field_name('name').text in (b'tests', b'help_tests'):
            return True
        cur = cur.parent
    return False


def collect(root, apply=False):
    entries = set()
    wrapped = 0
    for crate in LIBRARY_FOLDERS:
        for path in sorted((root/'crates'/crate/'src').rglob('*.rs')):
            if path.name == 'i18n.rs':
                continue
            data = path.read_bytes()
            tree = PARSER.parse(data).root_node
            edits = []
            for lit in walk(tree, skip_name_pools=(crate == 'map_core' and path.name == 'names.rs')):
                if lit.type not in ('string_literal', 'raw_string_literal'):
                    continue
                translated = value(lit)
                if not translated or not HAN.search(translated) or MARKUP.search(translated) or in_test(lit):
                    continue
                entries.add(translated)
                if not apply or crate != 'map_app':
                    continue
                start, end = lit.start_byte, lit.end_byte
                if display_argument(lit):
                    edits.append((start, end, b'crate::i18n::tr(' + data[start:end] + b')'))
                elif lit.parent.type == 'token_tree' and lit.parent.named_children[0] == lit:
                    macro = lit.parent.parent
                    identifier = macro.child_by_field_name('macro')
                    if identifier is not None and identifier.text == b'format' and display_argument(macro) and not FILE_SUFFIX.search(translated):
                        edits.append((identifier.start_byte, identifier.end_byte, b'crate::trf'))
            if apply and edits:
                for start, end, replacement in sorted(edits, reverse=True):
                    data = data[:start] + replacement + data[end:]
                path.write_bytes(data)
                wrapped += len(edits)
    return entries, wrapped


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--inventory',type=Path)
    cli.add_argument('--wrap',action='store_true')
    args = cli.parse_args()
    entries, count = collect(ROOT, args.wrap)
    for path in (ROOT/'docs/tutorial').rglob('lessons.json'):
        for chapter in json.loads(path.read_text(encoding='utf8')):
            entries.update(chapter[key] for key in ('title','goal','caption','check','help'))
            for step in chapter['steps']:
                entries.update([step['title'],step['body']])
    if args.inventory:
        args.inventory.parent.mkdir(parents=True, exist_ok=True)
        args.inventory.write_text(json.dumps(sorted(entries), ensure_ascii=False, indent=2)+'\n',encoding='utf8')
    print(f'{len(entries)} source messages, {count} wrapped calls')


if __name__ == '__main__':
    main()
