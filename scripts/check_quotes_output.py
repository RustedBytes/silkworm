#!/usr/bin/env python3
"""Assert the complete two-page offline QuotesSpider JSONL output."""
import json
from pathlib import Path
import sys


def main():
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('data/quotes.jl')
    items = [json.loads(line) for line in path.read_text().splitlines()]
    expected = [
        {'text': '"Offline quote one"', 'author': 'Author One', 'tags': ['offline', 'demo']},
        {'text': '"Offline quote two"', 'author': 'Author Two', 'tags': ['rust', 'crawler']},
        {'text': '"Offline quote page two"', 'author': 'Author Three', 'tags': ['page2']},
    ]
    assert sorted(items, key=lambda item: item['author']) == sorted(expected, key=lambda item: item['author']), items
    print('Verified all 3 quotes from 2 local pages.')


if __name__ == '__main__':
    main()
