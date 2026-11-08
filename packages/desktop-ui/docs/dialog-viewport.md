# DialogViewport

Wrap a native `Modal` body's content in `<DialogViewport>`. Short content is
centered; tall content scrolls from its first to last edge, including actions.
`inset` defaults to 16 pixels (allowed 0–128). Place a `SelectProvider` outside
this viewport so select portals are not clipped by the scroll surface.

This composes native ScrollView with fillViewport and a minimum-height column.
It has no window listeners, timers, platform branches, or fixed viewport size.
The entire dialog scrolls, including its heading. Keep domain lists virtualized
and give nested lists a bounded height; DialogViewport does not virtualize content.
