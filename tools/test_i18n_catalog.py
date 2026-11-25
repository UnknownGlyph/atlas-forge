import json
import unittest

from i18n_catalog import unique_object, validate


class CatalogueRules(unittest.TestCase):
    def test_keys_are_exact_and_duplicate_keys_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            json.loads('{"重复":"first","重复":"second"}', object_pairs_hook=unique_object)
        validate({'CPU': 'CPU', 'cpu': 'cpu'})
        validate({'按 <Enter>': 'Press <Enter>'})

    def test_fields_can_move_but_multiplicity_and_format_must_match(self):
        validate({'{a} {a} {n:.2}': '{n:.2} {a} {a}'})
        for invalid in ['{n} {a} {a}', '{n:.2} {a}', '{n:.2} {a} {b}', '{n:.2} {a} {a} }']:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate({'{a} {a} {n:.2}': invalid})

    def test_markup_and_blank_values_are_rejected(self):
        for messages in [{'<!DOCTYPE html>标题': 'Title'}, {'标题': '<style>text</style>'}, {'标题': ' '}, {'标题': None}]:
            with self.subTest(messages=messages), self.assertRaises(ValueError):
                validate(messages)
