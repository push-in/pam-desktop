<?php

declare(strict_types=1);
use Pam\Desktop\GridTracks;
use Pam\Desktop\Internal\ScopedStyleCompiler;
use Pam\Desktop\Internal\TemplateRenderer;
use Pam\Desktop\PropKey;
use Pam\Desktop\UI\Grid;
$tracks = GridTracks::fromCss('minmax(0, 1.8fr) minmax(300px, .9fr)');
$assert($tracks->toWire() === '0,1.8;300,0.9' && $tracks->count() === 2, 'Native grid preserves fractional weights and minima.');
$assert(GridTracks::fromWire($tracks->toWire())->toWire() === $tracks->toWire(), 'Grid tracks wire roundtrip.');
$assert(GridTracks::fromCss('repeat(3, minmax(0, 1fr))')->toWire() === '0,1;0,1;0,1', 'Fixed repeat expands bounded tracks.');
$grid = Grid::make()->tracks($tracks);
$assert($grid->properties()[PropKey::GridTracks->value] === $tracks->toWire() && $grid->properties()[PropKey::GridColumns->value] === 2, 'Fluent grid emits tracks and column count.');
$compiled = ScopedStyleCompiler::compileDeclarations('grid-template-columns: minmax(0,1.8fr) minmax(300px,0.9fr);', [], 'grid tracks');
$assert($compiled['gridTracks'] === $tracks->toWire() && $compiled['columns'] === '2', 'Scoped CSS retains grid semantics instead of just column count.');
foreach (['repeat(65,1fr)', 'minmax(-1px,1fr)', 'minmax(300px,0fr)', '1fr trailing', '0fr', 'minmax(auto,1fr)'] as $invalid) {
    try { GridTracks::fromCss($invalid); $assert(false, 'Invalid tracks rejected: '.$invalid); } catch (InvalidArgumentException) {}
}
echo "GRID_TRACKS_OK weights minima repeat bounded-parser wire CSS fluent\n";
$oldDesktop = \Pam\Desktop\RendererContract::fromJson('{"abi":1,"events":[80,2010],"properties":[543,2021],"protocol":[1,1]}');
$assert(!$oldDesktop->supportsProperty(PropKey::GridTracks->value), 'A previous desktop host cannot silently accept new grid tracks.');
try {
    \Pam\Desktop\RendererContract::use($oldDesktop);
    try { (new \Pam\Desktop\Internal\TreeEncoder())->encode($grid); $assert(false, 'Old renderer must reject grid tracks in encoder.'); }
    catch (\Pam\Desktop\ProtocolMismatchException $error) { $assert(str_contains($error->getMessage(), '2022'), 'Encoder identifies unsupported GridTracks.'); }
} finally { \Pam\Desktop\RendererContract::use(null); }
$boundTracks = \Pam\Desktop\Internal\TemplateCompiler::compile('<Grid :gridTracks="$tracks" />', 'grid-tracks.pam');
$rendered = TemplateRenderer::render($boundTracks, null, ['tracks'=>$tracks]);
$assert(($rendered->properties()[PropKey::GridTracks->value] ?? null) === $tracks->toWire(), 'Bound GridTracks object reaches native property without loss.');
