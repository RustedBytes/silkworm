#!/usr/bin/env python3
"""Compile Markdown Rust snippets against this checkout and run a local quickstart.

Statement fragments receive the context described in docs/README.md. Rust
blocks in architecture.md are intentionally labelled text pseudocode instead.
No external websites are fetched. Requires Python 3 and the project toolchain.
"""
from pathlib import Path
import argparse
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONTEXT = '''use silkworm::*;
use std::sync::Arc;
struct QuotesSpider;
impl Spider for QuotesSpider {
    async fn parse(&self, _: HtmlResponse<Self>) -> SpiderResult<Self> { Ok(vec![]) }
}
fn fixture() -> HtmlResponse<QuotesSpider> {
    Response {
        url: "http://localhost/page/1".into(), status: 200, headers: Headers::new(),
        body: bytes::Bytes::from_static(b"<h1>Fixture</h1>"),
        request: Request::get("http://localhost/page/1"),
    }.into_html(5_000_000)
}
'''

def check_links():
    for path in [ROOT / 'README.md', *sorted((ROOT / 'docs').glob('*.md'))]:
        text = path.read_text()
        if text.count('```') % 2:
            raise ValueError(f"Unclosed code fence: {path}")
        for target in re.findall(r'\]\(([^)]+)\)', text):
            if '://' in target:
                continue
            file, _, anchor = target.partition('#')
            destination = path.parent / file if file else path
            if not destination.exists():
                raise ValueError(f"Broken link: {path}: {target}")
            if anchor:
                headings = [re.sub(r'[^\w -]', '', h.lower()).replace(' ', '-')
                            for h in re.findall(r'^#+ (.+)$', destination.read_text(), re.M)]
                if anchor not in headings:
                    raise ValueError(f"Missing anchor: {path}: {target}")


def generate():
    modules = []
    quickstart = None
    for path in [ROOT / 'README.md', *sorted((ROOT / 'docs').glob('*.md'))]:
        for match in re.finditer(r'^```rust\n(.*?)^```', path.read_text(), re.M | re.S):
            code = match[1]
            line = path.read_text()[:match.start()].count('\n') + 1
            index = len(modules)
            if path == ROOT / 'README.md' and 'impl Spider for QuotesSpider' in code:
                quickstart = index
            declaration = re.search(r'(?m)^(?:pub )?(?:enum |struct |impl[< ]|(?:async )?fn main\()', code)
            if not declaration or re.search(r'(?m)^(let |Ok\(out\))', code):
                result = 'SpiderResult<QuotesSpider>' if 'Ok(out)' in code else 'SilkwormResult<()>'
                tail = '' if 'Ok(out)' in code else '\nOk(())'
                code = f'async fn fragment() -> {result} {{\nlet response = fixture();\n{code}{tail}\n}}'
            modules.append(f'// {path.relative_to(ROOT)}:{line}\nmod snippet_{index} {{\nuse super::*;\n{code}\n}}')
    assert quickstart is not None
    # Execute the exact README program, with only its demo URL bound to a local
    # server fixture. This checks fetching, selectors, item output and shutdown.
    original = modules[quickstart]
    modules[quickstart] = original.replace('struct QuotesSpider;', 'pub struct QuotesSpider;')
    offline = original.replace(f'mod snippet_{quickstart}', 'mod snippet_offline').replace('fn main()', 'pub fn main()').replace(
        'https://quotes.toscrape.com/', 'http://127.0.0.1:__DOC_PORT__/')
    modules.append(offline)
    runtime = f'''
#[test]
fn readme_quickstart_local() {{
    use std::io::{{Read, Write}};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {{
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut buffer = [0; 8192];
        stream.read(&mut buffer).unwrap();
        let body = "<div class='quote'><span class='text'>Local quote</span><small class='author'>Ada</small></div>";
        write!(stream, "HTTP/1.1 200 OK\\r\\nContent-Type: text/html\\r\\nContent-Length: {{}}\\r\\nConnection: close\\r\\n\\r\\n{{}}", body.len(), body).unwrap();
    }});
    // The request override is supplied by the generated source's runtime URL.
    DOC_PORT.store(port, std::sync::atomic::Ordering::SeqCst);
    snippet_offline::main().unwrap();
    server.join().unwrap();
}}
#[tokio::test]
async fn readme_parsing_and_pagination() {{
    let body = b"<div class='quote'><span class='text'>Local quote</span><small class='author'>Ada</small></div><li class='next'><a href='/page/2'>Next</a></li>";
    let response = Response {{
        url: "http://localhost/page/1".into(), status: 200, headers: Headers::new(),
        body: bytes::Bytes::from_static(body), request: Request::get("http://localhost/page/1"),
    }}.into_html(5_000_000);
    let outputs = snippet_{quickstart}::QuotesSpider.parse(response).await.unwrap();
    assert_eq!(outputs.len(), 2);
    match &outputs[0] {{
        SpiderOutput::Item(item) => assert_eq!(item, &serde_json::json!({{"text":"Local quote", "author":"Ada"}})),
        _ => panic!("expected item"),
    }}
    match &outputs[1] {{
        SpiderOutput::Request(request) => assert_eq!(request.url, "http://localhost/page/2"),
        _ => panic!("expected follow request"),
    }}
}}
static DOC_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
'''
    source = '#![allow(dead_code, unused_imports, unused_variables)]\n' + CONTEXT + '\n'.join(modules) + runtime
    source = source.replace('vec!["http://127.0.0.1:__DOC_PORT__/"]', 'vec![]')
    # start_urls borrows strings, so generate a start_requests override for the
    # local fixture instead of leaking an allocated URL into a borrowed vector.
    source = source.replace('fn start_urls(&self) -> Vec<&str> {\n        vec![]\n    }', '''async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(format!("http://127.0.0.1:{}/", DOC_PORT.load(std::sync::atomic::Ordering::SeqCst)))]
    }''')
    return source, len(modules) - 1

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--all-features', action='store_true',
                        help='include charset-detection (requires Rust >= 1.98)')
    args = parser.parse_args()
    optional_flags = ['--all-features'] if args.all_features else ['--features', 'xpath,cli-examples']
    check_links()
    source, count = generate()
    tests = ROOT / 'tests'
    tests.mkdir(exist_ok=True)
    # Unique temporary Cargo integration target, removed even on check failure.
    with tempfile.NamedTemporaryFile(mode='w', suffix='.rs', prefix='docs_check_', dir=tests) as test:
        test.write(source)
        test.flush()
        target = Path(test.name).stem
        for flags in [[], optional_flags, ['--no-default-features']]:
            subprocess.run(['cargo', 'test', '--locked', '--test', target, *flags], cwd=ROOT, check=True)
    print(f'Verified {count} Rust snippets in three feature configurations; local quickstart passed.')

if __name__ == '__main__':
    main()
