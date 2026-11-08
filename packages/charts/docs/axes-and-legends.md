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
));
DonutChart::make(200)->slices($slices)->legend(LegendPosition::Bottom)->legendInline()->theme($theme);
```

An inline legend (`legendInline()`) lays its items out left to right,
centred, and wraps them at the chart width; `legendSpacing()` is its row
height. Template themes accept the same metrics as `legendFontSize`,
`legendFontWeight`, `legendMarkerRadius`, `legendMarkerGap`, `legendItemGap`
and `legendValueGap` keys. The defaults (11 px medium labels, 4 px dots, 12 px
gaps) keep the previous look.
