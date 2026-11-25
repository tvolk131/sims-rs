#!/usr/bin/env python3
"""Migrates `.antics` v2 sources from the `@`-annotation / `tree` syntax to Rust-style
attributes and `#[bhav] fn` (docs/decisions/20261125-antics-v2-rust-like-syntax.md).

    migrate_v2_syntax.py FILE...        rewrite .antics files in place
    migrate_v2_syntax.py --rust FILE... rewrite the string literals of Rust sources in place

`.air` text is left alone: an `.air` tree header always names its numeric id
(`tree 4096 "Init"`), and its `@label`s never take the `@name(` / `name @ N` shapes below.
The migration is purely textual; `sims-antics fmt` gives the canonical layout afterwards.
"""

import re
import sys

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
# A format! placeholder can stand for an identifier in Rust test sources.
NAME = rf"(?:{IDENT}|\{{{IDENT}\}})"
INT = r"-?(?:0x[0-9A-Fa-f_]+|[0-9][0-9_]*)"
ANNOT = r"@[ \t]*(?:id|guard_id|context|frame)[ \t]*\([^()]*\)"

TREE = re.compile(
    rf"(?P<pre>(?:{ANNOT}[ \t]*(?:\n[ \t]*)?)*)"
    rf"\btree[ \t]+(?:(?P<ident>{NAME})[ \t]*)?(?:\"(?P<label>[^\"\n]*)\"[ \t]*)?"
    r"(?P<params>\((?:[^()]|\([^()]*\))*\))?[ \t]*"
    r"(?P<ret>->[^{;\n]*?)?[ \t]*(?=\{)"
)
RET_TEMP = re.compile(r"^->\s*(?:(Result)\s*<\s*(.*?)\s*@\s*temp\s*\[\s*(\d+)\s*\]\s*>|(.*?)\s*@\s*temp\s*\[\s*(\d+)\s*\])\s*$")
FN_RET = re.compile(
    rf"\bfn[ \t]+(?P<name>{NAME})[ \t]*(?P<params>\((?:[^()]|\([^()]*\))*\))[ \t]*"
    r"->[ \t]*(?:(?P<res>Result)[ \t]*<[ \t]*(?P<rty>[^@;{}<>]*?)[ \t]*@[ \t]*temp[ \t]*\[[ \t]*(?P<rt>\d+)[ \t]*\][ \t]*>"
    r"|(?P<ty>[^@;{}]*?)[ \t]*@[ \t]*temp[ \t]*\[[ \t]*(?P<t>\d+)[ \t]*\])"
)
PARAM_TEMP = re.compile(rf"(?P<name>\b{IDENT})[ \t]*:[ \t]*(?P<ty>[^,()@:]+?)[ \t]*@[ \t]*temp[ \t]*\[[ \t]*(?P<t>\d+)[ \t]*\]")
STATE_SLOT = re.compile(rf"\bstate[ \t]+(?P<name>{IDENT})[ \t]*@[ \t]*(?P<n>{INT})[ \t]*:")
BCON_ENTRY = re.compile(rf"(?P<name>\b{IDENT})[ \t]*@[ \t]*(?P<t>{INT})[ \t]*\[[ \t]*(?P<e>{INT})[ \t]*\]")
ADDR_ENTRY = re.compile(rf"(?P<name>\b{IDENT})[ \t]*@[ \t]*(?P<a>{INT})(?=[ \t]*[;,}}])")
FRAME = re.compile(r"@[ \t]*frame[ \t]*\((?P<body>[^()]*)\)")
SIMPLE = re.compile(r"@[ \t]*(?P<name>id|guard_id|context|slot)[ \t]*\((?P<body>[^()]*)\)")
# Prose mentions in comments: `@frame`, `@slot`, `@id`, `@context`.
MENTION = re.compile(r"`@(?P<name>id|guard_id|context|slot|frame)`")


def snake_case_ident(label: str) -> str:
    """Mirror of `parser::snake_case_ident`."""
    out = []
    pending_sep = False
    prev_lower = False
    for ch in label:
        if ch.isascii() and ch.isalnum():
            if ch.isupper() and prev_lower:
                pending_sep = True
            if pending_sep and out:
                out.append("_")
            pending_sep = False
            prev_lower = ch.islower() or ch.isdigit()
            out.append(ch.lower())
        else:
            pending_sep = True
            prev_lower = False
    s = "".join(out)
    if not s or s[0].isdigit():
        s = "tree_" + s
    return s


def frame_fields(body: str):
    fields = []
    for part in body.split(","):
        part = part.strip()
        if not part:
            continue
        if ":" in part:
            k, v = part.split(":", 1)
            fields.append(f"{k.strip()} = {v.strip()}")
        else:
            fields.append(part)
    return fields


def migrate_tree(m: re.Match) -> str:
    ident, label = m.group("ident"), m.group("label")
    if ident is None and label is None:
        return m.group(0)
    if ident is None:
        ident = snake_case_ident(label)
    args = []
    rest = []
    for a in re.finditer(r"@[ \t]*(\w+)[ \t]*\(([^()]*)\)", m.group("pre") or ""):
        name, body = a.group(1), a.group(2).strip()
        if name == "id":
            args.insert(0, f"id = {body}")
        elif name == "frame":
            rest.extend(frame_fields(body))
        elif name == "context":
            rest.append(f"context = {body}")
        else:
            rest.append(f"__unsupported_{name}")
    if label is not None and label != ident:
        args.append(f'label = "{label}"')
    args.extend(rest)
    ret = (m.group("ret") or "").strip()
    if ret:
        r = RET_TEMP.match(ret)
        if r:
            if r.group(1):
                ret = f"-> Result<{r.group(2).strip()}>"
                args.append(f"ret_temp = {r.group(3)}")
            else:
                ret = f"-> {r.group(4).strip()}"
                args.append(f"ret_temp = {r.group(5)}")
    attr = "#[bhav]" if not args else f"#[bhav({', '.join(args)})]"
    params = m.group("params") or "()"
    head = f"fn {ident}{params}" + (f" {ret}" if ret else "") + " "
    # Keep the attribute on its own line when the header started a line.
    start = m.start()
    line_start = m.string.rfind("\n", 0, start) + 1
    indent = m.string[line_start:start]
    if indent.strip() == "" and "\n" in m.string:
        return f"{attr}\n{indent}{head}"
    return f"{attr} {head}"


def migrate_fn_ret(m: re.Match) -> str:
    if m.group("res"):
        ret, t = f"Result<{m.group('rty').strip()}>", m.group("rt")
    else:
        ret, t = m.group("ty").strip(), m.group("t")
    return f"#[ret(temp = {t})] fn {m.group('name')}{m.group('params')} -> {ret}"


def migrate(text: str) -> str:
    text = TREE.sub(migrate_tree, text)
    text = FN_RET.sub(migrate_fn_ret, text)
    text = PARAM_TEMP.sub(lambda m: f"#[temp({m.group('t')})] {m.group('name')}: {m.group('ty').strip()}", text)
    text = STATE_SLOT.sub(lambda m: f"#[slot({m.group('n')})] state {m.group('name')}:", text)
    text = BCON_ENTRY.sub(lambda m: f"#[bcon({m.group('t')}, {m.group('e')})] {m.group('name')}", text)
    text = ADDR_ENTRY.sub(lambda m: f"#[addr({m.group('a')})] {m.group('name')}", text)
    text = FRAME.sub(lambda m: f"#[frame({', '.join(frame_fields(m.group('body')))})]", text)
    text = SIMPLE.sub(lambda m: f"#[{m.group('name')}({m.group('body')})]", text)
    text = MENTION.sub(lambda m: f"`#[{m.group('name')}]`", text)
    return text


RUST_STR = re.compile(r'(?P<raw>r(?P<h>#*)"(?P<rbody>.*?)"(?P=h))|(?P<norm>"(?P<nbody>(?:[^"\\]|\\.)*)")', re.S)


def migrate_rust(text: str) -> str:
    out = []
    pos = 0
    i = 0
    n = len(text)
    while i < n:
        # Skip line comments, char literals and lifetimes outside strings.
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        m = RUST_STR.match(text, i) if text[i] in 'r"' else None
        if m and (text[i] == '"' or (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_"))):
            out.append(text[pos:i])
            if m.group("raw") is not None:
                h = m.group("h")
                body = migrate(m.group("rbody"))
                if f'"{h}' in body:
                    raise SystemExit(f"raw string would close early at offset {i}")
                out.append(f'r{h}"{body}"{h}')
            else:
                body = m.group("nbody").replace('\\"', '"')
                body = migrate(body).replace('"', '\\"')
                out.append(f'"{body}"')
            pos = i = m.end()
            continue
        if text[i] == "'":
            # char literal like '"' or '\''
            m2 = re.match(r"'(?:\\.|[^'\\])'", text[i:])
            if m2:
                i += m2.end()
                continue
        i += 1
    out.append(text[pos:])
    return "".join(out)


def main(argv):
    rust = False
    files = []
    for a in argv:
        if a == "--rust":
            rust = True
        else:
            files.append(a)
    for f in files:
        src = open(f, encoding="utf-8").read()
        new = migrate_rust(src) if rust else migrate(src)
        if new != src:
            open(f, "w", encoding="utf-8").write(new)
            print(f"migrated {f}")


if __name__ == "__main__":
    main(sys.argv[1:])
