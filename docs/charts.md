# Charts

`pushinbr/pam-desktop-charts` (namespace `Pam\Desktop\Charts`) draws line, area, bar and donut charts, sparklines and progress rings. A chart is an immutable value turned into a scene of vector commands and painted by the native canvas of the backend (Cairo on GTK, Core Graphics on AppKit, Direct2D on Win32) through `pushinbr/pam-desktop-canvas`. Nothing is rasterized in PHP and no web view is involved.

It ships inside `pushinbr/pam-desktop`: nothing else to require.

```php
// index.php, before App::run()
use Pam\Desktop\Charts\Charts;

Charts::register();
```

`Charts::register()` registers the tags `<LineChart>`, `<AreaChart>`, `<BarChart>`, `<DonutChart>`, `<Sparkline>` and `<ProgressRing>`. A chart built in PHP is placed with [`<Slot :content>`](templates.md#slots) or with `->toElement()` in a `render()` method.

## A bar chart

```php
use Pam\Desktop\Charts\{BarChart, LegendPosition, Series};

$this->sales = BarChart::make(width: 430, height: 300)
    ->series(Series::make('Units', [12, 18, 9, 22])->color('#6366F1'))
    ->labels(['Jan', 'Feb', 'Mar', 'Apr'])
    ->barThickness(40)
    ->barRadius(8)
    ->grid(5)
    ->legend(LegendPosition::Bottom);
```

```html
<Slot :content="$sales" />
```

Every method returns a modified copy.

## Cartesian charts

`LineChart`, `AreaChart` and `BarChart` share:

| Method | Meaning |
| --- | --- |
| `series(Series)` | Adds a series: `Series::make($label, $values)->color('#RRGGBB')`. |
| `labels(array)` | The labels of the category axis. |
| `grid(int)` | Horizontal grid lines above the baseline, 0 to 20 (3 by default); 0 hides the grid. |
| `beginAtZero(bool)` | Whether the value axis includes 0. On by default for bar charts, off for line and area charts. |
| `axis(bool)`, `valueLabels(bool)`, `tooltip(bool)` | Axis, labels on the values, the tooltip of the highlighted point. |
| `legend(LegendPosition)` | `None`, `Top`, `Bottom` or `Right`. |
| `valueFormatter(Closure(float): string)` | Text of the axis ticks and tooltips. |
| `highlight(?int)`, `onSelect(Closure(?int))` | The point to emphasise; the handler gets the nearest index while the pointer is down. |

**Value axis.** The axis is chosen from the data, as Chart.js does: the minimum and maximum of every series (stacked totals included) are widened to "nice" ticks with 1-2-5 steps, so 980 to 2040 draws 800 to 2200. With `beginAtZero` the range always includes 0. A single value is widened by 5% on each side.

| Chart | Extra methods |
| --- | --- |
| `LineChart` | `curve(Curve::Linear \| Monotone \| Smooth)`, `tension(0.4)`, `lineWidth()`, `markers(bool, radius, ?background)`, `area(bool)`, `areaOpacity(start, end)` |
| `AreaChart` | A `LineChart` with the area on. |
| `BarChart` | `grouped()`, `stacked()`, `barRadius(4)`, `barThickness(float\|null)`: a fixed bar width (Chart.js `barThickness`), 1 to 400, centred in its slot and never wider than the slot; `null` restores the proportional width. |

## DonutChart

```php
use Pam\Desktop\Charts\{DonutChart, LegendPosition, Slice};

$this->share = DonutChart::make(264)
    ->slices([Slice::make('Card', 62)->color('#6366F1'), Slice::make('Cash', 38)->color('#22C55E')])
    ->thickness(40)
    ->center('100%', 'of sales')
    ->legend(LegendPosition::Bottom)
    ->legendInline();
```

`thickness` (18), `gap` (2) between slices, `center(title, subtitle)`, `legendValues(bool)` and `legendSpacing()`; `total()` returns the sum of the slices. `legendInline()` lays a top or bottom legend out in rows, centred and wrapping at the width of the chart (the Chart.js default); without it the legend has one row per slice.

## Sparkline and ProgressRing

```php
Sparkline::make(96, 32)->values([3, 5, 4, 8, 6])->area(true)->endMarker(true);
ProgressRing::make(56)->progress(0.72)->thickness(6)->label('72%');
```

## Themes

`ChartTheme::light()`, `ChartTheme::dark()` and `ChartTheme::make(...)` (or `fromArray()`) set the colours of the axes, grid, text and tooltip; `->palette(ChartPalette::categorical())` or `ChartPalette::sequential($color, $steps)` the colours of series without their own; `->legendStyle(new LegendStyle(...))` the legend. A chart takes a theme with `->theme($theme)`.

## In templates

The tags take the same options as attributes, with bound values for data:

```html
<LineChart width="328" height="160" :series="$series" :labels="$months" curve="monotone" markers="true" />
<DonutChart size="200" :slices="$slices" legend="bottom" legendInline="true" />
```

Attributes: `width`, `height`, `series` (or `values`), `labels`, `grid`, `beginAtZero`, `axis`, `valueLabels`, `tooltip`, `legend`, `highlight`, `valueFormatter`, `theme`; line and area: `curve`, `tension`, `lineWidth`, `markers`, `pointRadius`, `pointBackground`, `area`, `areaStartOpacity`, `areaEndOpacity`; bar: `stacked`, `barRadius`; donut: `size`, `slices`, `thickness`, `gap`, `title`, `subtitle`, `legendValues`, `legendInline`, `legendSpacing`; sparkline: `values`, `curve`, `area`, `endMarker`, `color`; progress ring: `size`, `progress`, `thickness`, `color`, `track`, `label`. `barThickness` is only available in PHP.

## The canvas library

`pushinbr/pam-desktop-canvas` is the drawing layer underneath, usable on its own: `Canvas` builds rectangles, rounded rectangles, circles, sectors, polygons, lines, arcs, dashed lines, `Path` (`moveTo`, `lineTo`, `curveTo`, `quadTo`, `close`) with solid or gradient fills, and single-line text, then `CanvasView::make($canvas->scene())->onPointer(...)->toElement()` shows it and reports pointer down, move and up. See [Media and web](media-and-web.md#canvas) for the `Canvas` element itself.

## Platforms

Paths, gradients and text are painted by all three backends. The charts were checked on Linux. On macOS and Windows the canvas code type-checks; it has not been compared on screen.

## Tests

```bash
php packages/charts/tests/run.php
php packages/canvas/tests/run.php
```
