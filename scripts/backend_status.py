#!/usr/bin/env python3
"""Score GameClient backends and redraw the README coverage graphic.

Gitea runs this on every push to main. The result is derived from the
impl blocks under src/bridge, not from a hand-maintained percentage.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from xml.sax.saxutils import escape

START = "<!-- backend-status:start -->"
END = "<!-- backend-status:end -->"

SKIP_CALLS = {
    "Ok",
    "Err",
    "Some",
    "None",
    "vec",
    "format",
    "unwrap_or",
    "unwrap_or_else",
    "map_err",
    "ok_or_else",
    "saturating_sub",
    "saturating_add",
    "default",
    "into",
    "to_string",
    "clone",
    "expect",
    "unwrap",
    "is_some",
    "is_none",
    "as_u64",
    "as_str",
    "insert",
    "from",
    "new",
    "with_arguments",
    "in_frame",
    "validate_token",
    "now_unix_ms",
}

METHOD_NAMES = (
    "status",
    "ping",
    "snapshot",
    "inspect",
    "call_function",
    "game_command",
    "feature_state",
    "poll_events",
    "read_live_frame",
)

LABELS = {
    "NativeClient": "Ruffle native",
    "LegacyPipeClient": "Ruffle legacy",
    "FlashClient": "Flash ActiveX",
}


@dataclass(frozen=True)
class Cap:
    id: str
    short: str
    title: str
    group: str  # state | commands


CAPS: tuple[Cap, ...] = (
    Cap("status", "status", "Status", "state"),
    Cap("ping", "ping", "Ping", "state"),
    Cap("snapshot", "snapshot", "Snapshot", "state"),
    Cap("reflected", "reflected", "Reflected snapshot", "state"),
    Cap("frame", "frame", "Live frame", "state"),
    Cap("navigation", "navigation", "Navigation", "state"),
    Cap("inspect", "inspect", "Inspect", "commands"),
    Cap("invoke", "invoke", "Invoke", "commands"),
    Cap("commands", "commands", "Commands", "commands"),
    Cap("features", "features", "Features", "commands"),
    Cap("events", "events", "Events", "commands"),
)


@dataclass
class Surface:
    name: str
    cells: dict[str, bool] = field(default_factory=dict)

    def group_counts(self, group: str) -> tuple[int, int]:
        ids = [cap.id for cap in CAPS if cap.group == group]
        got = sum(1 for cap_id in ids if self.cells.get(cap_id))
        return got, len(ids)


@dataclass
class Report:
    backends: list[Surface]

    def tally(self, group: str) -> tuple[int, int]:
        got = total = 0
        for backend in self.backends:
            part, whole = backend.group_counts(group)
            got += part
            total += whole
        return got, total


def code_mask(src: str) -> list[bool]:
    n = len(src)
    mask = [True] * n
    i = 0
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                mask[k] = False
            i = j
            continue
        if c == "/" and nxt == "*":
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                mask[k] = False
            i = j
            continue
        if c == "r" and _at_raw_string(src, i):
            j = _skip_raw_string(src, i)
            for k in range(i, j):
                mask[k] = False
            i = j
            continue
        if c == '"':
            j = _skip_string(src, i)
            for k in range(i, j):
                mask[k] = False
            i = j
            continue
        if c == "'":
            j = _skip_char_or_lifetime(src, i)
            if j > i + 1:
                for k in range(i, j):
                    mask[k] = False
            i = j
            continue
        i += 1
    return mask


def _at_raw_string(src: str, i: int) -> bool:
    if i > 0 and (src[i - 1].isalnum() or src[i - 1] == "_"):
        return False
    j = i + 1
    while j < len(src) and src[j] == "#":
        j += 1
    return j < len(src) and src[j] == '"'


def _skip_raw_string(src: str, i: int) -> int:
    j = i + 1
    hashes = 0
    while j < len(src) and src[j] == "#":
        hashes += 1
        j += 1
    if j >= len(src) or src[j] != '"':
        return i + 1
    j += 1
    closer = '"' + ("#" * hashes)
    end = src.find(closer, j)
    return len(src) if end < 0 else end + len(closer)


def _skip_string(src: str, i: int) -> int:
    j = i + 1
    while j < len(src):
        if src[j] == "\\":
            j += 2
            continue
        if src[j] == '"':
            return j + 1
        j += 1
    return len(src)


def _skip_char_or_lifetime(src: str, i: int) -> int:
    if i + 1 < len(src) and src[i + 1] == "\\":
        j = i + 2
        while j < len(src) and src[j] != "'":
            j += 1
        return min(len(src), j + 1)
    if i + 2 < len(src) and src[i + 2] == "'":
        return i + 3
    return i + 1


def code_text(src: str, mask: list[bool] | None = None) -> str:
    if mask is None:
        mask = code_mask(src)
    return "".join(ch if mask[i] else " " for i, ch in enumerate(src))


def extract_block(src: str, open_index: int, mask: list[bool]) -> tuple[str, int]:
    if open_index >= len(src) or src[open_index] != "{":
        raise ValueError("expected an opening brace")
    depth = 0
    i = open_index
    n = len(src)
    while i < n:
        if mask[i]:
            if src[i] == "{":
                depth += 1
            elif src[i] == "}":
                depth -= 1
                if depth == 0:
                    return src[open_index + 1 : i], i + 1
        i += 1
    raise ValueError("unbalanced brace")


def _next_code(src: str, mask: list[bool], i: int) -> int:
    while i < len(src) and not mask[i]:
        i += 1
    return i


def extract_methods(body: str) -> dict[str, str]:
    mask = code_mask(body)
    methods: dict[str, str] = {}
    i = 0
    n = len(body)
    depth = 0
    while i < n:
        if not mask[i]:
            i += 1
            continue
        if body[i] == "{":
            depth += 1
            i += 1
            continue
        if body[i] == "}":
            depth -= 1
            i += 1
            continue
        if depth == 0 and body.startswith("fn", i):
            prev = i - 1
            while prev >= 0 and not mask[prev]:
                prev -= 1
            if prev >= 0 and (body[prev].isalnum() or body[prev] == "_"):
                i += 1
                continue
            matched = re.match(r"fn\s+([A-Za-z0-9_]+)\s*\(", body[i:])
            if not matched:
                i += 1
                continue
            name = matched.group(1)
            brace = body.find("{", i + matched.end() - 1)
            if brace < 0:
                break
            inner, end = extract_block(body, brace, mask)
            if name in METHOD_NAMES:
                methods[name] = inner
            i = end
            continue
        i += 1
    return methods


def find_impls(src: str) -> list[tuple[str, dict[str, str]]]:
    mask = code_mask(src)
    found: list[tuple[str, dict[str, str]]] = []
    for matched in re.finditer(r"impl\s+GameClient\s+for\s+([A-Za-z0-9_]+)", src):
        if not mask[matched.start()]:
            continue
        brace = src.find("{", matched.end())
        body, _end = extract_block(src, brace, mask)
        found.append((matched.group(1), extract_methods(body)))
    return found


def find_fn_body(src: str, name: str) -> str | None:
    mask = code_mask(src)
    for matched in re.finditer(rf"\bfn\s+{re.escape(name)}\s*\(", src):
        if not mask[matched.start()]:
            continue
        brace = src.find("{", matched.end())
        if brace < 0:
            return None
        body, _end = extract_block(src, brace, mask)
        return body
    return None


def collapsed(src: str) -> str:
    return " ".join(code_text(src).split())


def read_expr(src: str, mask: list[bool], start: int) -> tuple[str, int]:
    i = _next_code(src, mask, start)
    while i < len(src) and mask[i] and src[i].isspace():
        i += 1
    if i < len(src) and mask[i] and src[i] == "{":
        inner, end = extract_block(src, i, mask)
        return inner, end
    paren = 0
    brace = 0
    begin = i
    while i < len(src):
        if not mask[i]:
            i += 1
            continue
        ch = src[i]
        if ch == "(":
            paren += 1
        elif ch == ")":
            paren = max(0, paren - 1)
        elif ch == "{":
            brace += 1
        elif ch == "}":
            if brace == 0 and paren == 0:
                return src[begin:i], i
            brace = max(0, brace - 1)
        elif ch == "," and paren == 0 and brace == 0:
            return src[begin:i], i + 1
        i += 1
    return src[begin:], len(src)


def try_inner_split(body: str) -> tuple[str, str] | None:
    mask = code_mask(body)
    head = collapsed(body)
    header = head.split("{", 1)[0]
    if head.startswith("let Some") and "self.inner" in header:
        return _split_let_else(body, mask)
    if head.startswith("match") and "self.inner" in header:
        return _split_match(body, mask)
    return None


def _split_let_else(body: str, mask: list[bool]) -> tuple[str, str] | None:
    else_at = re.search(r"\belse\b", code_text(body, mask))
    if not else_at:
        return None
    i = else_at.end()
    while i < len(body) and not (mask[i] and body[i] == "{"):
        i += 1
    if i >= len(body):
        return None
    none_arm, end = extract_block(body, i, mask)
    j = _next_code(body, mask, end)
    if j < len(body) and body[j] == ";":
        j = _next_code(body, mask, j + 1)
    return body[j:], none_arm


def _split_match(body: str, mask: list[bool]) -> tuple[str, str] | None:
    brace = body.find("{")
    while brace >= 0 and not mask[brace]:
        brace = body.find("{", brace + 1)
    if brace < 0:
        return None
    inner, _end = extract_block(body, brace, mask)
    inner_mask = code_mask(inner)
    arms: dict[str, str] = {}
    i = 0
    while i < len(inner):
        if not inner_mask[i]:
            i += 1
            continue
        matched = re.match(r"(Some|None)\b", inner[i:])
        if not matched or not _word_boundary_before(inner, inner_mask, i):
            i += 1
            continue
        kind = matched.group(1)
        arrow = code_text(inner, inner_mask).find("=>", i + matched.end())
        if arrow < 0:
            break
        arm, nxt = read_expr(inner, inner_mask, arrow + 2)
        arms[kind] = arm
        i = nxt
    if "Some" not in arms or "None" not in arms:
        return None
    return arms["Some"], arms["None"]


def _word_boundary_before(src: str, mask: list[bool], i: int) -> bool:
    prev = i - 1
    while prev >= 0 and not mask[prev]:
        prev -= 1
    return prev < 0 or not (src[prev].isalnum() or src[prev] == "_")


def split_if_let_target(body: str) -> tuple[str, str] | None:
    mask = code_mask(body)
    matched = re.search(r"if\s+let\s+Some\s*\(\s*target\s*\)", code_text(body, mask))
    if not matched:
        return None
    i = matched.end()
    while i < len(body) and not (mask[i] and body[i] == "{"):
        i += 1
    if i >= len(body):
        return None
    branch, end = extract_block(body, i, mask)
    j = _next_code(body, mask, end)
    if body.startswith("else", j):
        k = j + 4
        while k < len(body) and not (mask[k] and body[k] == "{"):
            k += 1
        other, _else_end = extract_block(body, k, mask)
        return branch, other
    return branch, body[end:]


def is_implemented(body: str) -> bool:
    code = code_text(body)
    if not code.strip():
        return False
    if re.search(r"\bOk\s*\(", code):
        return True
    if re.search(r"\breturn\s+true\b", code):
        return True
    if re.search(r"\bsuper\s*::", code):
        return True
    if re.search(r"\bcrate\s*::\s*features\s*::", code):
        return True
    if re.search(r"\b(?:self\s*\.\s*)?inner\s*\.\s*(?:feature_state|poll_events|game_command|status|ping|snapshot|inspect|call_function|read_live_frame)\s*\(", code):
        return True
    if re.search(r"\bself\s*\.\s*(?:feature_state|poll_events|game_command|status|ping|snapshot|inspect|call_function|read_live_frame)\s*\(", code):
        return True
    if "to_game_snapshot" in code or "read_into" in code or "json_command" in code:
        return True
    return False


def classify(body: str, dual: bool) -> tuple[bool, ...]:
    if not dual:
        return (is_implemented(body),)
    split = try_inner_split(body)
    if split is None:
        flag = is_implemented(body)
        return flag, flag
    some, none = split
    return is_implemented(some), is_implemented(none)


def live_implemented(body: str | None) -> bool:
    if body is None:
        return False
    code = code_text(body)
    if not code.strip():
        return False
    return "read_into" in code or "is_fresh" in code or re.search(r"\breturn\s+true\b", code) is not None


def keeps_navigation(text: str, file_text: str, seen: set[str] | None = None) -> bool:
    seen = set() if seen is None else seen
    code = code_text(text)
    if re.search(r"navigation\s*:\s*(?:Default|NavigationState)::default\s*\(", code):
        return False
    if "to_game_snapshot" in code:
        return True
    if re.search(r"navigation\s*:\s*(?:[A-Za-z_][A-Za-z0-9_]*\s*\.\s*)+navigation\b", code):
        return True
    for name in re.findall(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(", code):
        if name in SKIP_CALLS or name in seen:
            continue
        helper = find_fn_body(file_text, name)
        if helper is None:
            continue
        seen.add(name)
        if keeps_navigation(helper, file_text, seen):
            return True
    return False


def score_type(type_name: str, methods: dict[str, str], file_text: str) -> list[Surface]:
    dual = type_name == "FlashNativeClient"
    if dual:
        surfaces = [Surface("Flash Windows"), Surface("Flash Linux")]
    else:
        surfaces = [Surface(LABELS.get(type_name, type_name))]

    snapshot = methods.get("snapshot")
    reflected_src = snapshot
    default_src = snapshot
    if snapshot:
        parts = split_if_let_target(snapshot)
        if parts:
            reflected_src, default_src = parts

    values: dict[str, tuple[bool, ...]] = {}
    for name in ("status", "ping", "inspect", "call_function", "game_command", "feature_state", "poll_events"):
        body = methods.get(name)
        values[name] = tuple(False for _ in surfaces) if body is None else classify(body, dual)

    if reflected_src is None:
        values["reflected"] = tuple(False for _ in surfaces)
    else:
        values["reflected"] = classify(reflected_src, dual)
    if default_src is None:
        values["snapshot"] = tuple(False for _ in surfaces)
        nav = False
    else:
        values["snapshot"] = classify(default_src, dual)
        nav = keeps_navigation(default_src, file_text)
    values["navigation"] = tuple(nav for _ in surfaces)
    frame = live_implemented(methods.get("read_live_frame"))
    values["frame"] = tuple(frame for _ in surfaces)

    key_for = {
        "status": "status",
        "ping": "ping",
        "snapshot": "snapshot",
        "reflected": "reflected",
        "frame": "frame",
        "navigation": "navigation",
        "inspect": "inspect",
        "invoke": "call_function",
        "commands": "game_command",
        "features": "feature_state",
        "events": "poll_events",
    }
    for surface_index, surface in enumerate(surfaces):
        for cap in CAPS:
            flags = values[key_for[cap.id]]
            surface.cells[cap.id] = flags[surface_index if dual else 0]
    return surfaces


def build_report(src_dir: Path) -> Report:
    backends: list[Surface] = []
    order = {"NativeClient": 0, "LegacyPipeClient": 1, "FlashClient": 2, "FlashNativeClient": 3}
    impls: list[tuple[int, str, dict[str, str], str]] = []
    for path in sorted(src_dir.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        if "impl GameClient for" not in text:
            continue
        for type_name, methods in find_impls(text):
            impls.append((order.get(type_name, 50), type_name, methods, text))
    for _rank, type_name, methods, text in sorted(impls, key=lambda item: (item[0], item[1])):
        backends.extend(score_type(type_name, methods, text))
    if not backends:
        raise SystemExit(f"no GameClient implementations found under {src_dir}")
    return Report(backends)


def percent(got: int, total: int) -> str:
    if total <= 0:
        return "0%"
    value = 100.0 * got / total
    if abs(value - round(value)) < 1e-9:
        return f"{int(round(value))}%"
    return f"{value:.2f}%"


def text_width(text: str, size: float) -> float:
    return len(text) * size * 0.56


def render_svg(report: Report) -> str:
    backends = report.backends
    margin = 36
    col_gap = 16
    col_w = 198
    width = margin * 2 + len(backends) * col_w + (len(backends) - 1) * col_gap
    cols = 3
    pad_x = 10
    gap_x = 8
    square = (col_w - pad_x * 2 - gap_x * (cols - 1)) / cols
    label_size = 11
    slot_h = square + 6 + label_size + 10
    group_gap = 18
    state_caps = [cap for cap in CAPS if cap.group == "state"]
    cmd_caps = [cap for cap in CAPS if cap.group == "commands"]
    state_rows = (len(state_caps) + cols - 1) // cols
    cmd_rows = (len(cmd_caps) + cols - 1) // cols
    header_h = 46
    grid_h = state_rows * slot_h + group_gap + cmd_rows * slot_h
    top = 168
    note = (
        "* State is status, ping, snapshot, reflected snapshot, live frame, and navigation. "
        "Commands are inspect, invoke, manager commands, feature state, and events. "
        "Green is a working path in that backend. Gray is the trait default or an explicit unsupported result. "
        "Navigation is green when the snapshot keeps the client's NavigationState."
    )
    note_lines = _wrap(note, 150)
    foot_y = top + header_h + grid_h + 8
    height = foot_y + 36 + (len(note_lines) - 1) * 16 + 28

    bg = "#101216"
    green = "#3DDC84"
    gray = "#586270"
    ink = "#F4F7FB"
    muted = "#9AA3B2"
    chip = "#2A303A"
    parts: list[str] = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" viewBox="0 0 {width:.0f} {height:.0f}" role="img">',
        "<title>Backend coverage</title>",
        "<desc>Green cells are implemented GameClient paths. Gray cells are unsupported.</desc>",
        f'<rect width="{width:.0f}" height="{height:.0f}" fill="{bg}"/>',
        f'<text x="{margin}" y="52" fill="{ink}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="32" font-weight="600">Status</text>',
        f'<line x1="{margin}" y1="70" x2="{width - margin}" y2="70" stroke="#2C3340" stroke-width="1"/>',
    ]

    cursor = float(margin)
    for group, label in (("state", "state*"), ("commands", "commands*")):
        got, total = report.tally(group)
        label_w = text_width(label, 14) + 28
        pct = percent(got, total)
        pct_w = text_width(pct, 14) + 28
        parts.append(
            f'<rect x="{cursor:.1f}" y="90" width="{label_w:.1f}" height="32" rx="7" fill="{chip}"/>'
        )
        parts.append(
            f'<text x="{cursor + label_w / 2:.1f}" y="111" text-anchor="middle" fill="{ink}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="14">{escape(label)}</text>'
        )
        cursor += label_w + 4
        parts.append(
            f'<rect x="{cursor:.1f}" y="90" width="{pct_w:.1f}" height="32" rx="7" fill="{green}"/>'
        )
        parts.append(
            f'<text x="{cursor + pct_w / 2:.1f}" y="111" text-anchor="middle" fill="{bg}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="14" font-weight="700">{escape(pct)}</text>'
        )
        cursor += pct_w + 8
        fraction = f"{got}/{total}"
        parts.append(
            f'<text x="{cursor:.1f}" y="111" fill="{muted}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="14">{fraction}</text>'
        )
        cursor += text_width(fraction, 14) + 28

    for index, backend in enumerate(backends):
        x = margin + index * (col_w + col_gap)
        y = top
        got = sum(1 for cap in CAPS if backend.cells[cap.id])
        parts.append(
            f'<text x="{x}" y="{y + 16}" fill="{ink}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="15" font-weight="600">{escape(backend.name)}</text>'
        )
        frac = f"{got}/{len(CAPS)}"
        frac_fill = green if got == len(CAPS) else muted
        parts.append(
            f'<text x="{x + col_w}" y="{y + 16}" text-anchor="end" fill="{frac_fill}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="13">{frac}</text>'
        )
        _draw_cells(parts, backend, state_caps, x + pad_x, y + header_h, square, gap_x, slot_h, cols, green, gray, muted)
        cmd_y = y + header_h + state_rows * slot_h + group_gap
        _draw_cells(parts, backend, cmd_caps, x + pad_x, cmd_y, square, gap_x, slot_h, cols, green, gray, muted)

    legend = f'<rect x="{margin}" y="{foot_y}" width="12" height="12" rx="2" fill="{green}"/>'
    parts.append(legend)
    parts.append(
        f'<text x="{margin + 18}" y="{foot_y + 11}" fill="{muted}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="12">implemented</text>'
    )
    parts.append(f'<rect x="{margin + 120}" y="{foot_y}" width="12" height="12" rx="2" fill="{gray}"/>')
    parts.append(
        f'<text x="{margin + 138}" y="{foot_y + 11}" fill="{muted}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="12">unsupported</text>'
    )
    for line_index, line in enumerate(note_lines):
        parts.append(
            f'<text x="{margin}" y="{foot_y + 36 + line_index * 16}" fill="{muted}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="12">{escape(line)}</text>'
        )
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def _draw_cells(
    parts: list[str],
    backend: Surface,
    caps: list[Cap],
    origin_x: float,
    origin_y: float,
    square: float,
    gap_x: float,
    slot_h: float,
    cols: int,
    green: str,
    gray: str,
    muted: str,
) -> None:
    for index, cap in enumerate(caps):
        col = index % cols
        row = index // cols
        x = origin_x + col * (square + gap_x)
        y = origin_y + row * slot_h
        on = backend.cells[cap.id]
        fill = green if on else gray
        state = "implemented" if on else "unsupported"
        parts.append("<g>")
        parts.append(f"<title>{escape(backend.name)} — {escape(cap.title)} — {state}</title>")
        parts.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{square:.1f}" height="{square:.1f}" rx="4" fill="{fill}"/>')
        parts.append(
            f'<text x="{x + square / 2:.1f}" y="{y + square + 14:.1f}" text-anchor="middle" fill="{muted}" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif" font-size="11">{escape(cap.short)}</text>'
        )
        parts.append("</g>")


def _wrap(text: str, limit: int) -> list[str]:
    words = text.split()
    lines: list[str] = []
    current = ""
    for word in words:
        trial = word if not current else f"{current} {word}"
        if len(trial) > limit and current:
            lines.append(current)
            current = word
        else:
            current = trial
    if current:
        lines.append(current)
    return lines


def render_section(report: Report) -> str:
    headers = ["Capability", *[backend.name for backend in report.backends]]
    align = ["---"] * len(headers)
    rows = [headers, align]
    for cap in CAPS:
        rows.append(
            [cap.title, *("yes" if backend.cells[cap.id] else "no" for backend in report.backends)]
        )
    table = "\n".join("| " + " | ".join(row) + " |" for row in rows)
    return "\n".join(
        [
            START,
            "## Backend coverage",
            "",
            "`scripts/backend_status.py` reads each `GameClient` implementation and redraws this picture. Gitea runs it on every push to `main` and commits the picture when coverage changes.",
            "",
            "![Backend coverage](docs/backend-status.svg)",
            "",
            table,
            "",
            END,
        ]
    )


def splice_readme(text: str, report: Report) -> str:
    section = render_section(report)
    start = text.find(START)
    end = text.find(END)
    if start != -1 and end != -1 and end > start:
        prefix = text[:start].rstrip("\n")
        suffix = text[end + len(END) :].lstrip("\n")
        return prefix + "\n\n" + section + "\n\n" + suffix
    anchor = "\n## Backends\n"
    index = text.find(anchor)
    if index < 0:
        raise SystemExit("README is missing the coverage markers and the ## Backends heading")
    return text[:index].rstrip("\n") + "\n\n" + section + "\n" + text[index:]


def read_text(path: Path) -> tuple[str, str]:
    data = path.read_bytes()
    newline = "\r\n" if b"\r\n" in data else "\n"
    return data.decode("utf-8").replace("\r\n", "\n"), newline


def write_text(path: Path, text: str, newline: str) -> None:
    path.write_bytes(text.replace("\n", newline).encode("utf-8"))


def assert_parser() -> None:
    legacy = """
    impl GameClient for LegacyPipeClient {
        fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
            let target = target.unwrap_or("stage");
            let wire = self.pipe.json_command(&format!("STATE\\t{target}"))?;
            Ok(GameSnapshot { navigation: Default::default(), observed_at_unix_ms: wire })
        }
        fn status(&self) -> Result<ClientStatus> { super::status(&self.pipe, "legacy", |_| {}) }
        fn ping(&self) -> Result<()> { super::ping(&self.pipe) }
    }
    """
    legacy_surfaces = score_type(*_only(legacy), legacy)
    legacy_cells = legacy_surfaces[0].cells
    if not legacy_cells["snapshot"] or not legacy_cells["reflected"] or not legacy_cells["status"]:
        raise SystemExit("parser missed an implemented legacy path")
    if legacy_cells["navigation"] or legacy_cells["frame"] or legacy_cells["commands"]:
        raise SystemExit("parser marked a missing legacy path as implemented")

    dual = """
    fn decode_snapshot(value: Value) -> Result<GameSnapshot> {
        Ok(GameSnapshot { navigation: data.navigation })
    }
    impl GameClient for FlashNativeClient {
        fn feature_state(&self) -> Result<crate::ClientFeatureState> {
            match &self.inner {
                Some(inner) => inner.feature_state(),
                None => Err(ApiError::Unsupported("frames only".into())),
            }
        }
        fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
            if let Some(target) = target {
                let Some(inner) = &self.inner else {
                    return Err(ApiError::Unsupported("no targets".into()));
                };
                return inner.snapshot(Some(target));
            }
            Ok(frame.to_game_snapshot())
        }
        fn read_live_frame(&self, frame: &mut LiveFrame) -> bool {
            reader.read_into(frame)
        }
        fn ping(&self) -> Result<()> {
            let Some(inner) = &self.inner else {
                return Ok(());
            };
            inner.ping()
        }
    }
    """
    windows, linux = score_type(*_only(dual), dual)
    if not windows.cells["features"] or linux.cells["features"]:
        raise SystemExit("parser missed the inner/none feature split")
    if not windows.cells["reflected"] or linux.cells["reflected"]:
        raise SystemExit("parser missed the reflected-snapshot split")
    if not (windows.cells["snapshot"] and linux.cells["snapshot"] and linux.cells["navigation"]):
        raise SystemExit("parser missed the shared live snapshot")
    if not linux.cells["ping"] or not linux.cells["frame"]:
        raise SystemExit("parser missed a linux memory-only path")

    commented = """
    impl GameClient for NativeClient {
        // fn game_command(&self) { Ok(()) }
        fn status(&self) -> Result<ClientStatus> {
            super::status(&self.pipe, "native", |_| {})
        }
        fn ping(&self) -> Result<()> { Err(ApiError::Unsupported("no".into())) }
    }
    """
    native = score_type(*_only(commented), commented)[0]
    if native.cells["commands"] or native.cells["ping"] or not native.cells["status"]:
        raise SystemExit("parser treated a comment or an error return as a working path")


def _only(src: str) -> tuple[str, dict[str, str]]:
    impls = find_impls(src)
    if len(impls) != 1:
        raise SystemExit(f"fixture parsed {len(impls)} impls")
    return impls[0]


def print_report(report: Report) -> None:
    header = " ".join(f"{backend.name:>16}" for backend in report.backends)
    print(header)
    for cap in CAPS:
        marks = " ".join(
            f"{('yes' if backend.cells[cap.id] else 'no'):>16}" for backend in report.backends
        )
        print(f"{cap.title:<22}{marks}")
    for group in ("state", "commands"):
        got, total = report.tally(group)
        print(f"{group}: {percent(got, total)} {got}/{total}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="exit 1 when the graphic is stale")
    parser.add_argument("--root", type=Path, default=None, help="crate root (defaults to the repo)")
    args = parser.parse_args(argv)
    assert_parser()
    root = args.root if args.root is not None else Path(__file__).resolve().parents[1]
    report = build_report(root / "src")
    print_report(report)
    svg = render_svg(report)
    readme_path = root / "README.md"
    original, newline = read_text(readme_path)
    updated = splice_readme(original, report)
    svg_path = root / "docs" / "backend-status.svg"
    current_svg = svg_path.read_text(encoding="utf-8") if svg_path.exists() else ""
    if args.check:
        if current_svg != svg or updated != original:
            print("backend coverage graphic is stale; run python scripts/backend_status.py", file=sys.stderr)
            return 1
        print("backend coverage graphic is current")
        return 0
    svg_path.parent.mkdir(parents=True, exist_ok=True)
    svg_path.write_bytes(svg.encode("utf-8"))
    write_text(readme_path, updated, newline)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
