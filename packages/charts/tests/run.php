<?php

declare(strict_types=1);

use Pam\Desktop\Canvas\DisplayCommandKind;
use Pam\Desktop\Canvas\CanvasEventKind;
use Pam\Desktop\Canvas\CanvasScene;
use Pam\Desktop\Charts\AreaChart;
use Pam\Desktop\Charts\BarChart;
use Pam\Desktop\Charts\BarLayout;
use Pam\Desktop\Charts\ChartPalette;
use Pam\Desktop\Charts\ChartsPluginProvider;
use Pam\Desktop\Charts\ChartTheme;
use Pam\Desktop\Charts\Curve;
use Pam\Desktop\Charts\DonutChart;
use Pam\Desktop\Charts\Internal\Color;
use Pam\Desktop\Charts\Internal\LinearScale;
use Pam\Desktop\Charts\Internal\MonotoneCurve;
use Pam\Desktop\Charts\LegendPosition;
use Pam\Desktop\Charts\LineChart;
use Pam\Desktop\Charts\ProgressRing;
use Pam\Desktop\Charts\Series;
use Pam\Desktop\Charts\Slice;
use Pam\Desktop\Charts\Sparkline;
use Pam\Desktop\Element;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\TemplateRegistry;
use Pam\Desktop\UI\Canvas as CustomView;

require dirname(__DIR__, 3).'/vendor/autoload.php';
spl_autoload_register(static function (string $class): void {
    foreach (['Pam\\Desktop\\Charts\\' => dirname(__DIR__).'/src/', 'Pam\\Desktop\\Canvas\\' => dirname(__DIR__, 2).'/canvas/src/'] as $prefix => $root) {
        if (str_starts_with($class, $prefix)) {
            $file = $root.str_replace('\\', '/', substr($class, strlen($prefix))).'.php';
            if (is_file($file)) require $file;
        }
    }
});

/** @var array<string, Closure(): void> $tests */
$tests = [];
$test = static function (string $name, Closure $body) use (&$tests): void {
    $tests[$name] = $body;
};

function expect(bool $condition, string $message): void
{
    if (!$condition) {
        throw new RuntimeException($message);
    }
}

/** @return list<int> */
function kinds(CanvasScene $scene): array
{
    return array_map(static fn ($command): int => $command->kind->value, $scene->commands);
}

function countKind(CanvasScene $scene, DisplayCommandKind $kind): int
{
    return count(array_filter(kinds($scene), static fn (int $value): bool => $value === $kind->value));
}

function sampleLine(): LineChart
{
    return LineChart::make(width: 328, height: 160)
        ->theme(ChartTheme::dark(primary: '#19c5ff', ink: '#f4f7fa', muted: '#a0adbb', grid: '#2c3947', surface: '#19222c'))
        ->series(Series::make('Entradas', [120.5, 80, 140, 90, 200])->color('#19c5ff'))
        ->series(Series::make('Saídas', [40, 60, 30, 80, 50])->color('#ff8990'))
        ->labels(['Seg', 'Ter', 'Qua', 'Qui', 'Sex'])
        ->curve(Curve::Monotone)
        ->area(true)
        ->grid(3)
        ->valueFormatter(static fn (float $v): string => 'R$ ' . number_format($v, 0, ',', '.'))
        ->highlight(2);
}

$test('all coded variants are sequential integers', static function (): void {
    foreach ([Curve::cases(), LegendPosition::cases(), BarLayout::cases()] as $cases) {
        $values = array_map(static fn ($case) => $case->value, $cases);
        expect($values === range(1, count($values)), 'non-sequential enum');
    }
});

$test('colors normalize to #rrggbbaa and reject other forms', static function (): void {
    expect(Color::normalize('#abc') === '#aabbccff', 'short form');
    expect(Color::normalize('#19C5FF') === '#19c5ffff', 'six digits');
    expect(Color::normalize('#19c5ff80') === '#19c5ff80', 'eight digits');
    expect(Color::alpha('#19c5ff', 0.35) === '#19c5ff59', 'alpha replaced');
    expect(abs(Color::opacity('#00000000')) < 1e-9, 'opacity read');

    foreach (['19c5ff', '#12', 'red', '#gggggg', '#1234567'] as $invalid) {
        try {
            Color::normalize($invalid);
            throw new RuntimeException("accepted {$invalid}");
        } catch (InvalidArgumentException) {
        }
    }
});

$test('linear scales pick nice 1-2-5 ticks', static function (): void {
    expect(LinearScale::nice(0, 200, 3)->ticks() === [0.0, 100.0, 200.0], '0..200');
    expect(LinearScale::nice(0, 0.42, 4)->ticks() === [0.0, 0.2, 0.4, 0.6], '0..0.42');
    $scale = LinearScale::nice(-30, 70, 5);
    expect($scale->min === -40.0 && $scale->max === 80.0 && $scale->step === 20.0, '-30..70');
    $empty = LinearScale::nice(0, 0, 3);
    expect($empty->min === 0.0 && $empty->max === 1.0 && $empty->ticks() === [0.0, 0.5, 1.0], 'empty domain');
    expect(LinearScale::nice(0, 1234, 3)->ticks() === [0.0, 500.0, 1000.0, 1500.0], '0..1234');
    expect(LinearScale::nice(0, 10, 2)->position(5, 100, 0) === 50.0, 'position maps linearly');
});

$test('monotone curves never overshoot their neighbours', static function (): void {
    $points = [[0.0, 10.0], [10.0, 80.0], [20.0, 20.0], [30.0, 25.0], [40.0, 90.0], [50.0, 90.0], [60.0, 0.0]];
    $segments = MonotoneCurve::segments($points);
    expect(count($segments) === 6, 'one segment per interval');

    foreach ($segments as $i => [$c1x, $c1y, $c2x, $c2y, $x, $y]) {
        $low = min($points[$i][1], $points[$i + 1][1]);
        $high = max($points[$i][1], $points[$i + 1][1]);
        expect($c1y >= $low - 1e-9 && $c1y <= $high + 1e-9, "c1 of segment {$i} overshoots");
        expect($c2y >= $low - 1e-9 && $c2y <= $high + 1e-9, "c2 of segment {$i} overshoots");
        expect($c1x >= $points[$i][0] && $c2x <= $x, "controls of segment {$i} leave the x range");
        expect($y === $points[$i + 1][1], 'segment ends on the point');
    }

    expect(MonotoneCurve::segments([[0.0, 1.0]]) === [], 'single point has no segments');
});

$test('line chart scene snapshot', static function (): void {
    $scene = sampleLine()->scene();
    $kinds = kinds($scene);
    expect(count($kinds) === 23, 'command count changed: ' . count($kinds));
    expect($kinds[0] === DisplayCommandKind::FillRect->value, 'starts with the surface');
    expect(end($kinds) === DisplayCommandKind::Label->value, 'ends with an x label');
    expect(countKind($scene, DisplayCommandKind::GradientPath) === 2, 'one area per series');
    expect(countKind($scene, DisplayCommandKind::Path) === 2, 'one line per series');
    expect(countKind($scene, DisplayCommandKind::DashedLine) === 3, 'two grid lines and one crosshair');
    expect(countKind($scene, DisplayCommandKind::Circle) === 4, 'two markers with knockouts');
    expect(json_encode(json_decode($scene->toJson()))  === $scene->toJson(), 'json is stable');
    expect(CustomView::class === sampleLine()->view()->toElement()::class, 'renders a custom view');
});

$test('area chart is a line chart with the area on by default', static function (): void {
    $scene = AreaChart::make(200, 100)->series(Series::make('a', [1, 2, 3]))->scene();
    expect(countKind($scene, DisplayCommandKind::GradientPath) === 1, 'area present');
    expect(countKind($scene, DisplayCommandKind::Path) === 1, 'line present');
});

$test('line chart handles empty and single-point series', static function (): void {
    $empty = LineChart::make(200, 100)->series(Series::make('a', []))->labels(['x'])->scene();
    $kinds = kinds($empty);
    expect($kinds === [DisplayCommandKind::FillRect->value, DisplayCommandKind::Line->value, DisplayCommandKind::Label->value], 'baseline + empty label');
    expect(str_contains($empty->toJson(), \Pam\Desktop\I18n\FrameworkText::get('chart.empty')), 'muted empty label');

    $single = LineChart::make(200, 100)->series(Series::make('a', [42]))->area()->scene();
    expect(countKind($single, DisplayCommandKind::Circle) === 1, 'single point drawn as a marker');
    expect(countKind($single, DisplayCommandKind::Path) === 0, 'no path for a single point');
    expect(LineChart::make(200, 100)->series(Series::make('a', [42]))->indexAt(100) === 0, 'single point index');
});

$test('long series are split into several path commands under the spec cap', static function (): void {
    $values = array_map(static fn (int $i): float => sin($i / 9) * 50 + 60, range(0, 299));
    $scene = LineChart::make(400, 200)->series(Series::make('x', $values))->curve(Curve::Monotone)->area()->scene();
    expect(countKind($scene, DisplayCommandKind::Path) >= 4, 'line was chunked');
    expect(countKind($scene, DisplayCommandKind::GradientPath) === countKind($scene, DisplayCommandKind::Path), 'areas match lines');

    foreach ($scene->commands as $command) {
        foreach ($command->arguments as $argument) {
            expect(!is_string($argument) || strlen($argument) <= 4096, 'spec within the canvas cap');
        }
    }
});

$test('bar chart scene snapshot, grouped and stacked', static function (): void {
    $chart = BarChart::make(width: 328, height: 160)
        ->series(Series::make('Pix', [12, 18, 9, 22]))
        ->series(Series::make('Cartão', [8, 6, 11, 4]))
        ->labels(['Jan', 'Fev', 'Mar', 'Abr'])
        ->legend(LegendPosition::Bottom)
        ->highlight(1);

    $grouped = $chart->scene();
    $kinds = kinds($grouped);
    expect(countKind($grouped, DisplayCommandKind::Path) === 8, 'one bar per value');
    expect(countKind($grouped, DisplayCommandKind::RoundRect) === 1, 'highlight backdrop');
    expect(kinds($grouped)[0] === DisplayCommandKind::FillRect->value, 'starts with the surface');
    expect(end($kinds) === DisplayCommandKind::Label->value, 'ends with a label');
    expect(countKind($grouped, DisplayCommandKind::Label) >= 2 + 4 + 2, 'ticks, x labels, legend and highlight values');

    $stacked = $chart->stacked()->scene();
    expect(countKind($stacked, DisplayCommandKind::Path) === 4, 'one rounded top segment per category');
    expect(countKind($stacked, DisplayCommandKind::FillRect) === 1 + 4, 'surface plus one plain segment per category');
    $tall = $chart->series(Series::make('Boleto', [15, 15, 15, 15]));
    expect($tall->scale()->max === 30.0, 'grouped domain uses the single maximum');
    expect($tall->stacked()->scale()->max === 60.0, 'stacked domain uses sums');

    $thin = BarChart::make(200, 100)->series(Series::make('a', [5, 10]))->barThickness(8)->scene();
    $widths = [];
    foreach ($thin->commands as $command) {
        if ($command->kind === DisplayCommandKind::Path) $widths[] = $command;
    }
    expect(count($widths) === 2 && str_contains($thin->toJson(), '8'), 'fixed thickness bars drawn');
    expect(BarChart::make(200, 100)->series(Series::make('a', [5]))->barThickness(8)->scene()->toJson() !== BarChart::make(200, 100)->series(Series::make('a', [5]))->scene()->toJson(), 'barThickness changes the bar geometry');

    $negative = BarChart::make(200, 100)->series(Series::make('a', [-5, 10]))->scene();
    expect(countKind($negative, DisplayCommandKind::Path) === 2, 'negative bars drawn');
    expect(BarChart::make(200, 100)->series(Series::make('a', []))->scene()->commands !== [], 'empty bar chart draws');
});

$test('donut chart draws gapped sectors, center labels and a legend', static function (): void {
    $donut = DonutChart::make(size: 160)
        ->slices([Slice::make('Pix', 70.0)->color('#19c5ff'), Slice::make('Cartão', 30.0), Slice::make('Zero', 0)])
        ->thickness(18)
        ->gap(2)
        ->center(title: 'R$ 1.234', subtitle: 'este mês');
    $scene = $donut->scene();
    expect(countKind($scene, DisplayCommandKind::Sector) === 2, 'zero slices are skipped');
    expect(countKind($scene, DisplayCommandKind::Label) === 2, 'title and subtitle');

    $sweep = 0.0;
    foreach ($scene->commands as $command) {
        if ($command->kind === DisplayCommandKind::Sector) {
            $sweep += (float) $command->arguments[5];
            expect((float) $command->arguments[3] === 62.0, 'inner radius from thickness');
        }
    }
    expect($sweep < 360.0 && $sweep > 350.0, 'gaps trimmed from sweeps');

    $legend = $donut->legend(LegendPosition::Bottom);
    expect($legend->height() === 160.0 + 3 * DonutChart::LEGEND_ROW_HEIGHT + 8.0, 'legend grows the height');
    expect(countKind($legend->scene(), DisplayCommandKind::Circle) === 3, 'one swatch per slice');

    $inline = DonutChart::make(size: 240)
        ->slices([Slice::make('Pix', 70.0), Slice::make('Cartão', 30.0), Slice::make('Zero', 0)])
        ->legend(LegendPosition::Bottom)->legendInline()->legendValues(false);
    expect($inline->height() === 240.0 + DonutChart::LEGEND_ROW_HEIGHT + 8.0, 'inline legend fits short labels on one row');
    $narrow = DonutChart::make(size: 80)
        ->slices([Slice::make('Pix', 70.0), Slice::make('Cartão', 30.0), Slice::make('Zero', 0)])
        ->legend(LegendPosition::Bottom)->legendInline()->legendValues(false);
    expect($narrow->height() > 80.0 + DonutChart::LEGEND_ROW_HEIGHT + 8.0, 'inline legend wraps when the ring is narrow');
    expect(countKind($inline->scene(), DisplayCommandKind::Circle) === 3, 'inline legend keeps one swatch per slice');
    $circles = array_values(array_filter($inline->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Circle));
    expect((float) $circles[0]->arguments[1] === (float) $circles[2]->arguments[1] && (float) $circles[0]->arguments[0] < (float) $circles[2]->arguments[0], 'inline legend items share a row left to right');

    $empty = DonutChart::make(80)->scene();
    expect(countKind($empty, DisplayCommandKind::Sector) === 1 && str_contains($empty->toJson(), \Pam\Desktop\I18n\FrameworkText::get('chart.empty')), 'empty track');
});

$test('sparkline and progress ring snapshots', static function (): void {
    $spark = Sparkline::make(width: 96, height: 32)->values([3, 5, 4, 8, 6])->color('#19c5ff')->area(true)->curve(Curve::Monotone)->endMarker();
    $scene = $spark->scene();
    expect(kinds($scene) === [DisplayCommandKind::FillRect->value, DisplayCommandKind::GradientPath->value, DisplayCommandKind::Path->value, DisplayCommandKind::Circle->value, DisplayCommandKind::Circle->value], 'surface, area, line, marker knockout, marker');
    expect(kinds(Sparkline::make(96, 32)->scene()) === [DisplayCommandKind::FillRect->value, DisplayCommandKind::Line->value], 'empty sparkline draws a baseline');
    expect(countKind(Sparkline::make(96, 32)->values([7])->scene(), DisplayCommandKind::Circle) === 1, 'single value marker');

    $ring = ProgressRing::make(size: 56)->progress(0.42)->thickness(6)->color('#19c5ff')->track('#2c3947')->label('42%');
    $scene = $ring->scene();
    expect(kinds($scene) === [DisplayCommandKind::FillRect->value, DisplayCommandKind::Arc->value, DisplayCommandKind::Arc->value, DisplayCommandKind::Label->value], 'surface, track, arc, label');
    expect((float) $scene->commands[2]->arguments[4] === 151.2, 'sweep from progress');
    expect($scene->commands[2]->arguments[7] === 2, 'round cap');
    expect(kinds(ProgressRing::make(40)->progress(0)->scene()) === [DisplayCommandKind::FillRect->value, DisplayCommandKind::Arc->value], 'no arc at zero');
    $full = ProgressRing::make(40)->progress(7)->scene();
    expect(kinds($full) === [DisplayCommandKind::FillRect->value, DisplayCommandKind::Arc->value, DisplayCommandKind::Arc->value] && (float) $full->commands[2]->arguments[4] === 360.0, 'progress clamps to 1');
});

$test('highlight index resolves from pointer x', static function (): void {
    $line = sampleLine();
    [$px, , $pw] = (new ReflectionMethod($line, 'plotRect'))->invoke($line);
    expect($line->indexAt($px) === 0, 'first point');
    expect($line->indexAt($px + $pw) === 4, 'last point');
    expect($line->indexAt($px + $pw / 2 + 3) === 2, 'nearest middle point');
    expect($line->indexAt($px + $pw * 0.3) === 1, 'rounds to nearest');
    expect($line->indexAt(-50) === null && $line->indexAt(1000) === null, 'outside returns null');
    expect(LineChart::make(100, 50)->indexAt(50) === null, 'no data returns null');

    $bar = BarChart::make(328, 160)->series(Series::make('a', [1, 2, 3, 4]));
    [$px, , $pw] = (new ReflectionMethod($bar, 'plotRect'))->invoke($bar);
    expect($bar->indexAt($px + 1) === 0, 'first slot');
    expect($bar->indexAt($px + $pw * 0.6) === 2, 'third slot');
    expect($bar->indexAt($px + $pw) === 3, 'edge clamps to the last slot');
    expect($bar->indexAt($px - 1) === null, 'left of the plot');
});

$test('pointer events select the nearest index through the canvas view', static function (): void {
    $selected = [];
    $chart = sampleLine()->onSelect(static function (?int $index) use (&$selected): void {
        $selected[] = $index;
    });
    [$px, , $pw] = (new ReflectionMethod($chart, 'plotRect'))->invoke($chart);
    $events = $chart->toElement()->events();
    expect($events !== [], 'pointer handler wired');
    $handler = reset($events);
    expect($handler instanceof Closure, 'native event closure');
    $handler(Wire::map(['event' => CanvasEventKind::PointerDown->value, 'x' => $px + $pw, 'y' => 10.0]));
    $handler(Wire::map(['event' => CanvasEventKind::PointerMove->value, 'x' => $px, 'y' => 10.0]));
    $handler(Wire::map(['event' => CanvasEventKind::PointerUp->value, 'x' => $px, 'y' => 10.0]));
    $handler(Wire::map(['event' => CanvasEventKind::PointerCancel->value, 'x' => $px, 'y' => 10.0]));
    expect($selected === [4, 0, null], 'down/move select, up keeps, cancel clears: ' . json_encode($selected));
    expect(sampleLine()->toElement()->events() === [], 'static charts have no handler');
});

$test('revision changes with data and stays stable otherwise', static function (): void {
    $base = sampleLine();
    expect($base->revision() === sampleLine()->revision(), 'same inputs, same revision');
    expect($base->revision() !== $base->highlight(3)->revision(), 'highlight changes the revision');
    expect($base->revision() !== $base->series(Series::make('c', [1, 2, 3, 4, 5]))->revision(), 'series change the revision');
    expect($base->revision() > 0, 'revision is positive');
    expect($base->view()->toElement() instanceof Element, 'view renders');
});

$test('builders are immutable and validate inputs', static function (): void {
    $base = LineChart::make(100, 50);
    expect($base !== $base->area(), 'area clones');
    expect($base !== $base->series(Series::make('a', [1])), 'series clones');
    expect($base->scene()->commands !== [] && count($base->series(Series::make('a', [1]))->scene()->commands) > count($base->scene()->commands), 'clone carries the change');

    foreach ([
        static fn () => Series::make('a', [NAN]),
        static fn () => Series::make('a', [INF]),
        static fn () => Series::make('a', ['1']),
        static fn () => Series::make('a', [1])->color('blue'),
        static fn () => Slice::make('a', -1),
        static fn () => LineChart::make(0, 10),
        static fn () => LineChart::make(10, 5000),
        static fn () => DonutChart::make(100)->thickness(80),
        static fn () => ProgressRing::make(40)->progress(NAN),
        static fn () => BarChart::make(100, 50)->barRadius(-1),
        static fn () => ChartPalette::make(),
    ] as $i => $invalid) {
        try {
            $invalid();
            throw new RuntimeException("invalid input {$i} accepted");
        } catch (InvalidArgumentException) {
        }
    }
});

$test('themes and palettes assign series colors in fixed order', static function (): void {
    $theme = ChartTheme::dark(primary: '#ff00ff');
    expect($theme->palette->color(0) === '#ff00ffff', 'primary leads the palette');
    expect($theme->palette->color(1) === '#19c5ffff', 'then the categorical order');
    expect($theme->palette->color(99) === $theme->palette->color(99 % count($theme->palette->colors)), 'wraps');
    expect($theme->grid === Color::alpha('#f4f7fa', 0.12), 'grid defaults to 12% ink');
    expect(ChartTheme::light()->ink === '#101828ff', 'light theme');
    expect(ChartTheme::fromArray(['primary' => '#123456', 'ink' => '#fff'])->ink === '#ffffffff', 'from template array');
    expect(ChartTheme::fromArray(['primary' => '#123456'])->palette->color(0) === '#123456ff', 'array primary leads the palette');
    $sequential = ChartPalette::sequential('#19c5ff', 3);
    expect($sequential->colors === ['#19c5ff59', '#19c5ffac', '#19c5ffff'], 'light to dark ramp');
    expect($theme->seriesColor('#000000ff', 3) === '#000000ff', 'explicit colors win');
});

$test('registers the declarative chart tags', static function (): void {
    TemplateRegistry::reset();
    (new ChartsPluginProvider())->register();

    foreach (['LineChart', 'AreaChart', 'BarChart', 'DonutChart', 'Sparkline', 'ProgressRing'] as $tag) {
        expect(TemplateRegistry::factory($tag) instanceof Closure, "{$tag} missing");
    }

    $selected = [];
    $factory = TemplateRegistry::factory('LineChart');
    expect($factory !== null, 'line factory');
    $chart = $factory([
        'series' => [
            ['label' => 'Entradas', 'values' => [120.5, 80, 140, 90, 200], 'color' => '#19c5ff'],
            ['label' => 'Saídas', 'values' => ['40', '60', '30', '80', '50']],
        ],
        'labels' => ['Seg', 'Ter', 'Qua', 'Qui', 'Sex'],
        'width' => '328',
        'height' => '160',
        'curve' => 'monotone',
        'area' => 'true',
        'grid' => '3',
        'highlight' => '2',
        'legend' => 'top',
        'theme' => ['primary' => '#19c5ff', 'ink' => '#f4f7fa'],
        '__pamComponentEvents' => ['select' => static function (?int $index) use (&$selected): void {
            $selected[] = $index;
        }],
    ], [], null);
    expect($chart instanceof LineChart, 'line chart built from props');
    expect($chart->width() === 328.0 && $chart->height() === 160.0, 'string sizes coerced');
    $scene = $chart->scene();
    expect(countKind($scene, DisplayCommandKind::GradientPath) === 2, 'area="true" coerced');
    expect(countKind($scene, DisplayCommandKind::Circle) === 4 + 2, 'highlight markers plus legend swatches');
    $element = $chart->toElement();
    expect($element instanceof Element && $element->events() !== [], '@select wired');
    $events = $element->events();
    $handler = reset($events);
    expect($handler instanceof Closure, 'event closure');
    $handler(Wire::map(['event' => 1, 'x' => 400.0, 'y' => 1.0]));
    expect($selected === [null], 'select event forwarded');

    $area = TemplateRegistry::factory('AreaChart');
    expect($area !== null && countKind($area(['values' => [1, 2, 3]], [], null)->scene(), DisplayCommandKind::GradientPath) === 1, 'area tag defaults to area');

    $bar = TemplateRegistry::factory('BarChart');
    expect($bar !== null, 'bar factory');
    $barChart = $bar(['series' => [['label' => 'a', 'values' => '1,2,3'], ['label' => 'b', 'values' => '3,2,1']], 'stacked' => 'true', 'labels' => 'x,y,z'], [], null);
    expect($barChart instanceof BarChart && $barChart->scale()->max === 4.0, 'stacked="true" coerced');

    $donut = TemplateRegistry::factory('DonutChart');
    expect($donut !== null, 'donut factory');
    $donutChart = $donut(['size' => '120', 'thickness' => '14', 'title' => 'R$ 10', 'subtitle' => 'hoje', 'slices' => [['label' => 'Pix', 'value' => '70', 'color' => '#19c5ff'], ['label' => 'Cartão', 'value' => 30]]], [], null);
    expect($donutChart instanceof DonutChart && countKind($donutChart->scene(), DisplayCommandKind::Sector) === 2, 'donut from props');

    $spark = TemplateRegistry::factory('Sparkline');
    expect($spark !== null, 'sparkline factory');
    $sparkline = $spark(['values' => '3, 5, 4, 8', 'area' => 'true', 'curve' => 'monotone', 'color' => '#19c5ff'], [], null);
    expect($sparkline instanceof Sparkline && countKind($sparkline->scene(), DisplayCommandKind::GradientPath) === 1, 'sparkline from props');

    $ring = TemplateRegistry::factory('ProgressRing');
    expect($ring !== null, 'ring factory');
    $progress = $ring(['size' => '56', 'progress' => '0.42', 'thickness' => '6', 'label' => '42%'], [], null);
    expect($progress instanceof ProgressRing && kinds($progress->scene()) === [DisplayCommandKind::FillRect->value, DisplayCommandKind::Arc->value, DisplayCommandKind::Arc->value, DisplayCommandKind::Label->value], 'ring from props');

    TemplateRegistry::reset();
});

$test('accepts packed ARGB integer colors from templates', static function (): void {
    expect(\Pam\Desktop\Charts\Internal\Color::fromArgb(0xFF19C5FF) === '#19c5ffff', 'ARGB integers convert to #rrggbbaa.');
    expect(\Pam\Desktop\Charts\Internal\Color::fromArgb(0x8019C5FF) === '#19c5ff80', 'ARGB alpha is preserved.');
    expect(\Pam\Desktop\Charts\Internal\Props::color(0xFF19C5FF) === '#19c5ffff', 'Props::color accepts integers.');
    expect(\Pam\Desktop\Charts\Internal\Props::color('#19c5ff') === '#19c5ffff', 'Props::color accepts hex strings.');
    expect(\Pam\Desktop\Charts\Internal\Props::color('oops') === null, 'Props::color ignores non-colors.');
    $series = \Pam\Desktop\Charts\Internal\Props::series([['label' => 'A', 'values' => [1, 2], 'color' => 0xFF19C5FF]]);
    expect(count($series) === 1, 'Series parsed from an array with an integer color.');
    $theme = \Pam\Desktop\Charts\ChartTheme::fromArray(['primary' => 0xFF19C5FF, 'ink' => '#fff']);
    expect($theme->primary === '#19c5ffff', 'Theme accepts integer colors.');
});

$test('desktop line styling and hover tooltip preserve plot geometry', static function (): void {
    $base = sampleLine()->curve(Curve::Smooth)->tension(0.36)->lineWidth(2.5)->markers(true, 3, '#0f172a')->areaOpacity(0.22, 0.01)->tooltip();
    $selected = $base->highlight(2);
    $plot = new ReflectionMethod($base, 'plotRect');
    expect($plot->invoke($base) === $plot->invoke($selected), 'Hover tooltip must not move the plot.');
    expect(str_contains($selected->scene()->toJson(), 'Entradas:'), 'Tooltip carries the selected series label and value.');
    expect(countKind($base->scene(), DisplayCommandKind::Circle) >= 10, 'All points have visible outlined markers.');
    $native = json_decode(\Pam\Desktop\Canvas\Internal\DesktopCommands::encode($selected->scene()), true, flags: JSON_THROW_ON_ERROR);
    expect(count(array_filter($native, static fn (array $command): bool => $command['kind'] === 5 && str_contains($command['path'], 'C'))) > 0, 'Smooth lines reach the native renderer as cubic paths.');
});

$test('right-side donut legends do not stretch the ring vertically', static function (): void {
    $chart = DonutChart::make(160)->slices([Slice::make('Pix', 2), Slice::make('Cartão', 1)])->legend(LegendPosition::Right)->legendValues(false)->legendSpacing(30);
    expect($chart->width() === 300.0 && $chart->height() === 160.0, 'Right legend reserves horizontal room.');
    expect(!str_contains($chart->scene()->toJson(), '%'), 'Percentage labels can be omitted.');
});

$test('value axis fits the data like Chart.js unless it begins at zero', static function (): void {
    $sales = Series::make('Vendas', [980, 1500, 2040]);
    $line = LineChart::make(320, 160)->series($sales)->grid(7);
    $scale = $line->scale();
    expect($scale->min === 800.0 && $scale->max === 2200.0 && $scale->step === 200.0, 'line axis covers 800..2200 by 200');
    $zero = $line->beginAtZero()->scale();
    expect($zero->min === 0.0 && $zero->max >= 2040.0, 'beginAtZero keeps 0 on the axis');
    $bar = BarChart::make(320, 160)->series($sales)->grid(7)->scale();
    expect($bar->min === 0.0, 'bar charts begin at zero by default');
    expect(BarChart::make(320, 160)->series($sales)->beginAtZero(false)->grid(7)->scale()->min === 800.0, 'bar charts can fit their data');
    $flat = LineChart::make(320, 160)->series(Series::make('A', [500, 500]))->scale();
    expect($flat->min < 500.0 && $flat->max > 500.0 && $flat->min > 0.0, 'a single value is widened around itself');
    $negative = LineChart::make(320, 160)->series(Series::make('A', [-30, 40]))->scale();
    expect($negative->min <= -30.0 && $negative->max >= 40.0, 'ranges crossing 0 keep it');
    $json = $line->scene()->toJson();
    expect(str_contains($json, '"800"') || str_contains($json, '800'), 'the lowest tick is labelled');
    $props = ChartsPluginProvider::lineChart(['values' => [980, 2040], 'beginAtZero' => true], true);
    expect($props->scale()->min === 0.0, 'templates accept beginAtZero');
});

$test('chart labels carry the theme font family to the desktop canvas', static function (): void {
    $chart = BarChart::make(300, 200)->series(Series::make('Qtd', [3, 5]))->labels(['A', 'B'])->theme(ChartTheme::dark()->fontFamily("'Inter', Arial, sans-serif"));
    $json = \Pam\Desktop\Canvas\Internal\DesktopCommands::encode($chart->scene());
    expect(str_contains($json, '"family":"\'Inter\', Arial, sans-serif"'), 'labels are encoded with the CSS font list');
    $plain = \Pam\Desktop\Canvas\Internal\DesktopCommands::encode(BarChart::make(300, 200)->series(Series::make('Qtd', [3, 5]))->labels(['A', 'B'])->scene());
    expect(!str_contains($plain, '"family"'), 'without a family the platform UI font stays implicit');
    expect(ChartTheme::fromArray(['fontFamily' => 'Inter, sans-serif'])->axisStyle(new \Pam\Desktop\Charts\AxisStyle())->fontFamily === 'Inter, sans-serif', 'template themes set the font family and keep it');
});

$test('fonts without built-in metrics use the host measurement', static function (): void {
    $estimate = \Pam\Desktop\Charts\Internal\Text::measure('Ação', 12.0, 400, 'Fira Sans');
    expect(abs($estimate - \Pam\Desktop\Charts\Internal\Text::width('Ação', 12.0)) < 1e-9, 'without a host the estimate stays');
    \Pam\Desktop\System\TextMeasure::remember('Ação', 12.0, 400, 'Fira Sans', 25.5);
    expect(\Pam\Desktop\Charts\Internal\Text::measure('Ação', 12.0, 400, 'Fira Sans') === 25.5, 'a host width replaces the estimate');
    expect(abs(\Pam\Desktop\Charts\Internal\Text::measure('45', 12.0, 400, 'Arial') - 1139 * 2 / 2048 * 12) < 1e-9, 'known metrics stay exact');
});

$test('inline donut legends are native text measured by the host', static function (): void {
    $slices = [Slice::make('Lanches', 70.0), Slice::make('Bebidas', 30.0)];
    $theme = ChartTheme::dark()->fontFamily('Inter, Arial, sans-serif')->legendStyle(new \Pam\Desktop\Charts\LegendStyle(fontSize: 12.0, markerRadius: 8.49, markerGap: 6.0, itemGap: 20.0, markerBox: 12.0));
    $chart = DonutChart::make(264)->slices($slices)->legend(LegendPosition::Bottom)->legendInline()->legendValues(false)->legendSpacing(35)->theme($theme);
    $texts = [];
    $walk = static function (\Pam\Desktop\Element $element) use (&$walk, &$texts): void {
        if ($element->kind() === \Pam\Desktop\NodeKind::Text) $texts[] = $element->properties();
        foreach ($element->children() as $child) if ($child instanceof \Pam\Desktop\Element) $walk($child);
    };
    $walk($chart->toElement());
    expect(count($texts) === 2, 'one native label per slice');
    expect(in_array('Inter, Arial, sans-serif', array_map('strval', $texts[0]), true), 'labels carry the theme font family');
    expect(str_contains($chart->scene()->toJson(), 'Lanches'), 'scene() keeps the full drawing for non-native consumers');
    $texts = [];
    $walk(DonutChart::make(264)->slices($slices)->legend(LegendPosition::Right)->theme($theme)->toElement());
    expect($texts === [], 'other legends stay on the canvas');
});

$test('cartesian axes take their metrics from the theme', static function (): void {
    $plain = BarChart::make(430, 306)->series(Series::make('Qtd', [42, 31, 25, 18]))->labels(['A', 'B', 'C', 'D']);
    $kinds = static fn (BarChart $chart): array => array_count_values(array_map(static fn ($c) => $c->kind->name, $chart->scene()->commands));
    $axis = new \Pam\Desktop\Charts\AxisStyle(fontSize: 12.0, labelGap: 11.0, gridDash: 0.0, paddingRight: 0.0);
    $chartJs = $plain->theme(ChartTheme::dark()->axisStyle($axis));
    expect(!isset($kinds($chartJs)['DashedLine']) && isset($kinds($plain)['DashedLine']), 'gridDash 0 draws solid grid lines; the default stays dashed');
    $labels = array_values(array_filter($chartJs->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Label && $c->arguments[0] === 'A'));
    expect((float) $labels[0]->arguments[3] === 12.0, 'axis label size comes from the theme');
    $bars = static fn (BarChart $chart): float => max(array_map(static fn ($c) => (float) $c->arguments[1], array_filter($chart->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Label && $c->arguments[0] === 'D')));
    expect($bars($chartJs) > $bars($plain), 'a zero right padding lets the plot reach the edge');
    $fromProps = ChartTheme::fromArray(['axisFontSize' => '12', 'axisGridDash' => 0, 'axisPaddingRight' => 0]);
    expect($fromProps->axis->fontSize === 12.0 && $fromProps->axis->gridDash === 0.0 && $fromProps->axis->paddingRight === 0.0 && $fromProps->axis->labelGap === 6.0, 'template themes set axis metrics');
    expect(ChartTheme::dark()->legendStyle(new \Pam\Desktop\Charts\LegendStyle())->axisStyle($axis)->palette(\Pam\Desktop\Charts\ChartPalette::categorical('#6366F1'))->axis === $axis, 'axis metrics survive other theme changes');
});

$test('donut legends take their metrics from the theme', static function (): void {
    $slices = [Slice::make('Pix', 70.0), Slice::make('Cartão', 30.0), Slice::make('Dinheiro', 10.0)];
    $theme = ChartTheme::dark()->legendStyle(new \Pam\Desktop\Charts\LegendStyle(fontSize: 12.0, markerRadius: 6.0, itemGap: 20.0));
    $chart = DonutChart::make(300)->slices($slices)->legend(LegendPosition::Bottom)->legendInline()->legendValues(false)->theme($theme);
    $circles = array_values(array_filter($chart->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Circle));
    expect((float) $circles[0]->arguments[2] === 6.0, 'marker radius comes from the theme');
    $labels = array_values(array_filter($chart->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Label && $c->arguments[0] === 'Pix'));
    expect((float) $labels[0]->arguments[3] === 12.0, 'label size comes from the theme');
    $narrowTheme = ChartTheme::dark()->legendStyle(new \Pam\Desktop\Charts\LegendStyle(fontSize: 20.0, itemGap: 40.0));
    expect(DonutChart::make(300)->slices($slices)->legend(LegendPosition::Bottom)->legendInline()->legendValues(false)->theme($narrowTheme)->height() > $chart->height(), 'larger metrics wrap and grow the legend');
    expect(ChartTheme::fromArray(['ink' => '#ffffff'], $theme)->legend->fontSize === 12.0, 'template themes keep the legend metrics');
    $fromProps = ChartTheme::fromArray(['legendFontSize' => 13, 'legendFontWeight' => '400', 'legendItemGap' => '20']);
    $boxed = DonutChart::make(300)->slices($slices)->legend(LegendPosition::Bottom)->legendInline()->legendValues(false)
        ->theme(ChartTheme::dark()->legendStyle(new \Pam\Desktop\Charts\LegendStyle(fontSize: 12.0, markerRadius: 8.49, markerGap: 6.0, itemGap: 20.0, markerBox: 12.0)));
    $boxCircles = array_values(array_filter($boxed->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Circle));
    $boxLabels = array_values(array_filter($boxed->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Label && $c->arguments[0] === 'Pix'));
    expect(abs((float) $boxLabels[0]->arguments[1] - (float) $boxCircles[0]->arguments[0] - 12.0) < 1e-6, 'a marker box centres the dot and places the label after box and gap');
    expect(ChartTheme::fromArray(['legendMarkerBox' => '12'])->legend->markerBox === 12.0 && ChartTheme::fromArray([])->legend->markerBox === null, 'template themes set the marker box');
    expect($fromProps->legend->fontSize === 13.0 && $fromProps->legend->itemGap === 20.0 && $fromProps->legend->fontWeight === \Pam\Desktop\Canvas\FontWeight::Regular, 'template themes set legend metrics');
});

$test('Chart.js layouts: fitted axes and framed doughnuts', static function (): void {
    // Chart.js measures with the chart font: Arial 11px values.
    expect(abs(\Pam\Desktop\Charts\Internal\Text::measure('Aguard. Pagamento', 11, 600, "'Helvetica Neue', Arial") - 103.9038) < 0.001, 'Arial bold advances');
    expect(abs(\Pam\Desktop\Charts\Internal\Text::measure('R$ 2.200', 11, 400, 'Arial') - 44.6445) < 0.001, 'Arial regular advances');
    $theme = ChartTheme::fromArray(['fontFamily' => 'Arial', 'axisFontSize' => 11, 'axisFitEdgeLabels' => true, 'axisTickLength' => 8, 'axisTickPadding' => 3,
        'axisPaddingTop' => 0, 'axisPaddingRight' => 0, 'axisPaddingBottom' => 0, 'axisPaddingLeft' => 0, 'axisXLabelWeight' => 600, 'axisBorderColor' => '#0000001A']);
    expect($theme->axis->fitEdgeLabels && $theme->axis->xLabelWeight === \Pam\Desktop\Canvas\FontWeight::SemiBold && $theme->axis->borderColor !== null, 'template keys');
    $chart = \Pam\Desktop\Charts\LineChart::make(920, 236.8)->series(Series::make('Vendas', [980, 1220, 880, 1430, 1630, 2010, 1840]))
        ->labels(['1 de nov.', '2 de nov.', '3 de nov.', '4 de nov.', '5 de nov.', '6 de nov.', '7 de nov.'])->theme($theme);
    $labels = array_values(array_filter($chart->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Label && str_ends_with((string) $c->arguments[0], 'nov.')));
    expect((int) $labels[0]->arguments[5] === \Pam\Desktop\Canvas\TextAlign::Center->value && (int) $labels[6]->arguments[5] === \Pam\Desktop\Canvas\TextAlign::Center->value, 'edge labels stay centred on their points');
    expect(abs((float) $labels[6]->arguments[1] - (920 - 47.6792 / 2 - 3)) < 0.01, 'the plot ends half the last label (plus padding) before the edge');
    $donut = DonutChart::make(160)->slices([Slice::make('A', 1.0), Slice::make('B', 1.0)])->legend(LegendPosition::Right)->legendValues(false)
        ->frame(439, 220.8)->cutout(0.74)->cornerRadius(6)->spacing(2)->border(1, '#0F172A4D')->theme($theme);
    expect($donut->width() === 439.0 && $donut->height() === 220.8, 'a frame sets the canvas size');
    $paths = array_values(array_filter($donut->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Path));
    expect(count($paths) === 4, 'each slice is a filled and an outlined path');
    expect(str_contains((string) $paths[0]->arguments[0], 'C'), 'rounded slices are curves');
});

$test('Chart.js layouts: offset bar scales and host-measured labels', static function (): void {
    // Electron 37 + Chart.js 4.5.1 (Reports.vue, 430x306): chartArea 27.34765625, 10.2, 430, 277.6; bar base 277.1.
    $theme = ChartTheme::dark()->fontFamily("'Inter', Arial, sans-serif")->axisStyle(new \Pam\Desktop\Charts\AxisStyle(fontSize: 12.0, gridDash: 0.0,
        paddingTop: 0.0, paddingRight: 0.0, paddingBottom: 0.0, paddingLeft: 0.0, fitEdgeLabels: true, tickLength: 8.0, tickPadding: 3.0));
    $bar = BarChart::make(430, 306)->series(Series::make('Q', [42, 31, 25, 18]))->labels(['Hambúrguer', 'Suco', 'Batata', 'Brownie'])
        ->barThickness(40)->barRadius(8)->grid(9)->theme($theme);
    [$px, $py, $pw, $ph] = (new ReflectionMethod($bar, 'plotRect'))->invoke($bar);
    expect(abs($px - 27.34765625) < 1e-6 && abs($py - 10.2) < 1e-6 && abs($px + $pw - 430) < 1e-6 && abs($py + $ph - 277.6) < 1e-6, 'chartArea of an offset category scale');
    $narrow = BarChart::make(120, 100)->series(Series::make('Q', [1, 2]))->labels(['Hambúrguer artesanal', 'X'])->theme($theme);
    [$nx] = (new ReflectionMethod($narrow, 'plotRect'))->invoke($narrow);
    expect($nx > 27.0, 'a first label wider than its band pushes the plot right');
    $path = array_values(array_filter($bar->scene()->commands, static fn ($c) => $c->kind === DisplayCommandKind::Path))[0];
    expect(str_contains((string) $path->arguments[0], '277.43'), 'bars stop half the baseline grid line short, inflated 0.33px');
    $stacked = BarChart::make(430, 306)->stacked()->series(Series::make('A', [20, 10]))->series(Series::make('B', [10, 5]))->labels(['a', 'b'])
        ->barThickness(40)->barRadius(0)->grid(9)->theme($theme);
    $rects = array_values(array_filter($stacked->scene()->commands, static fn ($c) => in_array($c->kind, [DisplayCommandKind::Path, DisplayCommandKind::FillRect], true)));
    $first = $rects[1];
    expect($first->kind === DisplayCommandKind::FillRect && abs((float) $first->arguments[2] - 40.66) < 1e-6, 'stacked segments inflate 0.33px per side');
    $element = \Pam\Desktop\Charts\LineChart::make(920, 236.8)->series(Series::make('V', [1, 2]))->integerBitmap()->toElement();
    $props = $element->properties();
    expect(($props[\Pam\Desktop\PropKey::CanvasBitmap->value] ?? null) === true && ($props[\Pam\Desktop\PropKey::Height->value] ?? null) === 236.8
        && !isset($props[\Pam\Desktop\PropKey::ScaleY->value]), 'integer bitmaps keep the CSS size and ask the host for a snapped 1:1 bitmap');
    // The host's shaped width (kerning) wins over the advance table once known.
    \Pam\Desktop\System\TextMeasure::remember('7 de nov.', 11, 600, 'Arial', 46.86279296875);
    expect(\Pam\Desktop\Charts\Internal\Text::measure('7 de nov.', 11, 600, 'Arial') === 46.86279296875, 'host width wins');
});

$failed = 0;

foreach ($tests as $name => $body) {
    try {
        $body();
        fwrite(STDOUT, "PASS {$name}\n");
    } catch (Throwable $error) {
        $failed++;
        fwrite(STDERR, "FAIL {$name}: {$error->getMessage()} ({$error->getFile()}:{$error->getLine()})\n");
    }
}

fwrite(STDOUT, count($tests) . " tests, {$failed} failures\n");
exit($failed === 0 ? 0 : 1);
