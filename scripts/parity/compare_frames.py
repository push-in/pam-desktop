#!/usr/bin/env python3
"""Compares a PAM Desktop frame dump (PAM_DESKTOP_DUMP_FRAMES) with the
Chromium boxes of electron-dom.cjs, matching texts in reading order.

    compare_frames.py native.json chromium.json [--x-min 216] [--tolerance 0.02] [--all]

For every text found in both, prints the native layout box (LayoutUnit), the
Chromium box and the difference of each edge, and flags edges that land on
a different device pixel (what a screenshot comparison sees). Exit status 1
when any flagged edge remains, so it can gate a parity check.
"""
import argparse
import json
import sys
import unicodedata
from collections import defaultdict


def key(text):
    text = unicodedata.normalize('NFC', text or '')
    return ' '.join(text.split()).casefold()


def edges(rect):
    x, y, w, h = rect
    return x, y, x + w, y + h


def pixel(value, scale):
    # Blink and the scene both snap an edge with round-half-up.
    return int((value * scale) + 0.5)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('native')
    parser.add_argument('chromium')
    parser.add_argument('--x-min', type=float, default=None, help='ignore boxes left of this x (e.g. a shared sidebar)')
    parser.add_argument('--tolerance', type=float, default=0.02, help='layout differences below this are equal (px)')
    parser.add_argument('--all', action='store_true', help='print matched texts without differences too')
    parser.add_argument('--dom-box', choices=('content', 'border'), default='content',
                        help="Chromium box to compare: 'content' (padding and border removed, default) or 'border'")
    parser.add_argument('--edges', default='left,top,right,bottom',
                        help='edges to check, comma separated (e.g. top,bottom: a native Text box often spans its'
                             ' column while the DOM element is shrink-wrapped, so left/right differ without any'
                             ' pixel moving)')
    options = parser.parse_args()
    native = json.load(open(options.native, encoding='utf-8'))
    chromium = json.load(open(options.chromium, encoding='utf-8'))
    scale = float(native.get('scale', 1.0))

    def keep(rect):
        return options.x_min is None or rect[0] >= options.x_min

    natives = defaultdict(list)
    for node in native['nodes']:
        if node.get('text') and node['kind'] in ('Text', 'Button') and keep(node['layout']):
            natives[key(node['text'])].append(node)
    doms = defaultdict(list)
    for node in chromium['nodes']:
        if keep(node['rect']):
            doms[key(node['text'])].append(node)
            if node.get('ownText') and key(node['ownText']) != key(node['text']):
                doms[key(node['ownText'])].append(node)

    rows = []
    unmatched = []
    for text, items in natives.items():
        candidates = doms.get(text, [])
        items = sorted(items, key=lambda node: (node['layout'][1], node['layout'][0]))
        candidates = sorted(candidates, key=lambda node: (node['rect'][1], node['rect'][0]))
        for index, node in enumerate(items):
            if index >= len(candidates):
                unmatched.append(node)
                continue
            dom = candidates[index]
            box = dom.get('content', dom['rect']) if options.dom_box == 'content' else dom['rect']
            dom = dict(dom, rect=box)
            own = node.get('content') or node['layout']
            if options.dom_box == 'border':
                own = node['layout']
            node = dict(node, layout=own)
            ne, de = edges(own), edges(box)
            delta = [a - b for a, b in zip(ne, de)]
            shifted = [pixel(a, scale) != pixel(b, scale) for a, b in zip(ne, de)]
            rows.append((node, dom, delta, shifted))

    rows.sort(key=lambda row: (row[1]['rect'][1], row[1]['rect'][0]))
    flagged = 0
    names = ('left', 'top', 'right', 'bottom')
    wanted = {name.strip() for name in options.edges.split(',')}
    for index, (node, dom, delta, shifted) in enumerate(rows):
        delta = [value if name in wanted else 0.0 for name, value in zip(names, delta)]
        shifted = [shift and name in wanted for name, shift in zip(names, shifted)]
        rows[index] = (node, dom, delta, shifted)
    for node, dom, delta, shifted in rows:
        moved = any(abs(value) > options.tolerance for value in delta)
        if any(shifted):
            flagged += 1
        if not (moved or options.all):
            continue
        marks = ' '.join(
            f"{name}{'*' if shift else ''}={value:+.3f}"
            for name, value, shift in zip(names, delta, shifted)
            if abs(value) > options.tolerance or shift
        )
        x, y, w, h = node['layout']
        print(f"{node['text'][:40]!r:44} native y={y:9.4f} h={h:8.4f}  chromium y={dom['rect'][1]:9.4f} h={dom['rect'][3]:8.4f}  {marks or 'equal'}")
    print(f"\n{len(rows)} texts matched, {flagged} with an edge on another pixel (*), {len(unmatched)} native texts unmatched")
    return 1 if flagged else 0


if __name__ == '__main__':
    sys.exit(main())
