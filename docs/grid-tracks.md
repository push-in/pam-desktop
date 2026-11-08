# Fractional native grid tracks

PAM Desktop supports bounded fractional tracks in scoped CSS:

```css
.form-layout {
    grid-template-columns: minmax(0, 1.8fr) minmax(300px, .9fr);
    gap: 12px;
}
```

Track weights are proportional after subtracting gutters. A track whose fractional share is below its minimum is fixed at that minimum; remaining width is redistributed to the other tracks. When minima exceed available space, their authored minimum is preserved, as in CSS. Use responsive rules to choose fewer tracks on narrow screens.

Supported grammar: positive `<number>fr`, `minmax(<non-negative number>[px|dp], <positive number>fr)` and bounded integer `repeat(count, track)`. At most 64 tracks; dimensions/weights are finite and at most 1,000,000. Unsupported intrinsic tracks (`auto`, `min-content`, `max-content`, percentages, subgrid) fail explicitly rather than silently becoming equal-width columns.

PHP API: `Grid::make(...)->tracks(GridTracks::fromCss('2fr 1fr'))`, or `Style(gridTracks: GridTracks::fromCss(...))`. Native tags accept `:gridTracks="$tracks"`; serialized form is `minimum,weight;minimum,weight`. The engine derives the column count from the explicit tracks and shares the widths between placement and intrinsic height measurement. Existing `GridTemplate` responsive plans retain priority; `GridMinColumnWidth` still reduces columns and uses equal tracks after such a reduction.

Wire property `GridTracks = 2022` follows the existing desktop table. All three desktop backends advertise the common wire ceiling through the shared runtime. A host advertising the old 2021 ceiling is rejected by `RendererContract`/`TreeEncoder` before sending this property. Update the native host and SDK together. The closed core table remains unchanged.
