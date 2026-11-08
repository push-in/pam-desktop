<?php

declare(strict_types=1);

use Pam\Desktop\Align;
use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\PropKey;
use Pam\Desktop\Style;
use Pam\Desktop\System\Timers;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\VirtualizedList;

require __DIR__.'/../../vendor/autoload.php';

/**
 * A two-thousand-row list: PHP only builds the rows near the viewport and
 * the host asks for more as the user scrolls.
 */
final class Lists extends Component
{
    private const int ROWS = 2000;

    #[State]
    public float $offset = 0.0;

    #[State]
    public int $endReached = 0;

    #[State]
    public int $request = 0;

    #[State]
    public float $target = 0.0;

    #[State]
    public bool $refreshing = false;

    #[State]
    public int $refreshed = 0;

    public function render(): Column
    {
        $label = new Style(fontSize: 13, textColor: 0xFF6B7280);

        return Column::make(
            Text::make('Lists')->style(new Style(fontSize: 26, fontWeight: 700, textColor: 0xFF111827)),
            Text::make(sprintf('%d rows · offset %d · end reached %d× · refreshed %d× (F5)', self::ROWS, (int) $this->offset, $this->endReached, $this->refreshed))->style($label),
            Row::make(
                Button::make('Top')->onPress(function (): void {
                    $this->target = 0.0;
                    $this->request++;
                })->style(new Style(flexGrow: 1)),
                Button::make('Row 1000')->onPress(function (): void {
                    $this->target = 1000 * 56.0;
                    $this->request++;
                })->style(new Style(flexGrow: 1)),
                Button::make('End')->onPress(function (): void {
                    $this->target = self::ROWS * 56.0;
                    $this->request++;
                })->style(new Style(flexGrow: 1)),
            )->style(new Style(gap: 12)),
            VirtualizedList::of(
                range(1, self::ROWS),
                static fn (int $row): Row => Row::make(
                    Text::make((string) $row)->style(new Style(
                        width: 44,
                        height: 32,
                        borderRadius: 8,
                        backgroundColor: $row % 2 === 0 ? 0xFFDBEAFE : 0xFFEDE9FE,
                        textColor: 0xFF1E3A8A,
                        fontSize: 13,
                        fontWeight: 700,
                        textAlign: \Pam\Desktop\TextAlignment::Center,
                    )),
                    Column::make(
                        Text::make("Row number {$row}")->style(new Style(fontSize: 15, fontWeight: 600, textColor: 0xFF111827)),
                        Text::make('Built on demand by PHP, mounted as native views')->style(new Style(fontSize: 12, textColor: 0xFF6B7280)),
                    )->style(new Style(flexGrow: 1)),
                )->style(new Style(height: 56, paddingHorizontal: 14, gap: 12, alignItems: Align::Center, borderBottomWidth: 1, borderColor: 0xFFF3F4F6)),
                static fn (int $row): string => "row-{$row}",
            )
                ->rowHeight(56)
                ->scrollRequest($this->request, targetOffset: $this->target)
                ->onScroll(function (string $offset): void {
                    $this->offset = (float) $offset;
                })
                ->onEndReached(function (): void {
                    $this->endReached++;
                })
                // F5 or Ctrl+R refreshes: desktops have no pull gesture.
                ->refreshing($this->refreshing)
                ->refreshColors(0xFF2563EB)
                ->refreshProgressBackgroundColor(0xFFFFFFFF)
                ->refreshIndicatorSize(\Pam\Desktop\RefreshIndicatorSize::Large)
                ->onRefresh(function (): void {
                    $this->refreshing = true;
                    Timers::after(1500, function (): void {
                        $this->refreshing = false;
                        $this->refreshed++;
                    });
                })
                ->style(new Style(flexGrow: 1, borderRadius: 14, backgroundColor: 0xFFFFFFFF, borderWidth: 1, borderColor: 0xFFE5E7EB, overflow: \Pam\Desktop\Overflow::Hidden)),
        )->style(new Style(padding: 24, gap: 12, flexGrow: 1, backgroundColor: 0xFFF3F4F6));
    }
}

App::run(new Lists());
