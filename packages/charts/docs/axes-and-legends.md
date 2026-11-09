# Value axes and legends

## Value axis

The value axis of `LineChart`, `AreaChart` and `BarChart` follows Chart.js's
linear scale. Line and area charts fit their data: the axis runs from the
smallest to the largest value, widened outwards to nice 1-2-5 steps (980..2040
with `grid(10)` draws 800..2200 by 200, as Chart.js does with its default of
11 ticks). Bar charts begin at zero. `beginAtZero()` changes either default:

```php
LineChart::make(320, 160)->series($sales)->beginAtZero();      // starts at 0
BarChart::make(320, 160)->series($sales)->beginAtZero(false);  // fits the bars
```

`grid($n)` is the most tick intervals the axis may use (3 by default). A
range that crosses zero always includes it, and the solid baseline is drawn
at zero or at the range edge nearest to it. A single value is widened by 5%
each way. In templates: `<AreaChart beginAtZero="true" grid="10" … />`.

## Axis metrics

Cartesian charts take their tick labels, grid and padding from the theme:

```php
$theme = ChartTheme::dark()->axisStyle(new AxisStyle(
    fontSize: 12,        // tick and category labels (Chart.js 12)
    labelGap: 11,        // value labels to plot (Chart.js tick length 8 + padding 3)
    xLabelHeight: 25,    // band under the plot for the category labels
    gridDash: 0,         // solid grid lines like Chart.js; 3 (default) dashes them
    paddingRight: 0,     // Chart.js plots reach the right edge
));
```

`paddingTop`, `paddingRight`, `paddingBottom` and `paddingLeft` (8 by
default) are the space between the canvas edge and the plot or its labels.
Template themes accept `axisFontSize`, `axisLabelGap`, `axisXLabelHeight`,
`axisGridDash` and `axisPaddingTop|Right|Bottom|Left`. The defaults keep the
previous look.

`ChartTheme::fontFamily("Inter, Arial, sans-serif")` (template key
`fontFamily`) sets the CSS font list of every label; generic families resolve
as in text nodes, and an empty list keeps the platform UI font.

## Legends

Legend text and markers take their metrics from the theme:

```php
$theme = ChartTheme::dark()->legendStyle(new LegendStyle(
    fontSize: 12,        // label and percentage
    fontWeight: FontWeight::Medium,
    markerRadius: 3,     // the colour dot
    markerGap: 6,        // dot to label
    itemGap: 20,         // between inline items (Chart.js `padding`)
    valueGap: 6,         // label to percentage
    markerBox: 12,       // width the dot takes in an inline item (Chart.js `boxWidth`)
));
DonutChart::make(200)->slices($slices)->legend(LegendPosition::Bottom)->legendInline()->theme($theme);
```

An inline legend (`legendInline()`) lays its items out left to right,
centred, and wraps them at the chart width; `legendSpacing()` is its row
height. Template themes accept the same metrics as `legendFontSize`,
`legendFontWeight`, `legendMarkerRadius`, `legendMarkerGap`, `legendItemGap`,
`legendValueGap` and `legendMarkerBox` keys. Chart.js `usePointStyle` draws a
dot of radius `boxHeight·√2/2` centred in a `boxWidth` box, so its 12 px legend
is `markerRadius: 8.49, markerBox: 12, markerGap: 6, itemGap: 20`. The defaults (11 px medium labels, 4 px dots, 12 px
gaps) keep the previous look.

## Chart.js layouts

`AxisStyle(fitEdgeLabels: true)` lays a cartesian chart out like Chart.js: the
value labels and the edge category labels are measured by the host
(`text.measure`: the family resolved and shaped like Chromium, kerning
included, as canvas `measureText`); until the host answers, advance tables for
Arial/Helvetica/Liberation Sans and DejaVu Sans or the estimate. The plot
starts after `widest + tickLength + 2·tickPadding` and the category axis pads
like Chart.js `_calculatePadding`: half the first/last label plus
`tickPadding`, minus the tick's own offset from the plot edge (bar charts are
`offset`, their ticks sit in the middle of equal bands), scaled by
`width / (width - offset)`. The category band is
`tickLength + 2·tickPadding + 1.2·font`; value and category labels use
`textBaseline: 'middle'` with Blink's em-box middle (OS/2 typo metrics
normalised to the font size, LayoutUnits). Bars in this mode stop half the
baseline grid line short of it and, with a fixed `barThickness`, grow 0.33px on
each side (Chart.js `inflateAmount: 'auto'`); bar corners are circular.
Verified against `chart.chartArea` and the bar elements of Electron 37 +
Chart.js 4.5.1 (Reports 430x306: 27.348, 10.2, 430, 277.6).
`tickLength` extends the grid lines left of the plot, `xLabelWeight` sets the
category label weight and `borderColor` draws the axis lines. Template keys:
`axisFitEdgeLabels`, `axisTickLength`, `axisTickPadding`, `axisXLabelWeight`,
`axisBorderColor`; legends accept `legendColor`.

`DonutChart::frame($width, $height)` is the Chart.js doughnut: a right legend
takes its measured width, the ring fills and centres in the rest, rows are
centred at `legendSpacing()` pitch. `cutout()`, `cornerRadius()` (`borderRadius`),
`spacing()` and `border()` follow Chart.js `pathArc`. Template props:
`frameWidth`, `frameHeight`, `cutout`, `cornerRadius`, `spacing`,
`borderWidth`, `borderColor`.

## Integer bitmaps

`integerBitmap()` (template `:integerBitmap="true"`) keeps the chart at its
fractional CSS size and asks the host for an HTML-canvas bitmap
(`PropKey::CanvasBitmap`): Chart.js' `floor(size)` pixels, painted 1:1 into
the pixel-snapped box. The previous emulation stretched the bitmap to the
fractional size (×1.0036 for 220 → 220.8), which blurred the chart and moved
the legend of the Electron dashboard 0.2 px down.
