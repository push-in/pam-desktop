<?php

declare(strict_types=1);

use Pam\Desktop\Animation\Animation;
use Pam\Desktop\Animation\Easing;
use Pam\Desktop\AnimationEasing;
use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\MotionPreset;
use Pam\Desktop\PropKey;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\View;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Motion driven by the host's frame clock: implicit transitions, springs,
 * endless programs and shimmer placeholders. PHP only describes them.
 */
final class MotionDemo extends Component
{
    #[State]
    public bool $moved = false;

    #[State]
    public int $bounces = 0;

    public function render(): Column
    {
        $label = new Style(fontSize: 13, textColor: 0xFF6B7280);
        $card = new Style(padding: 16, gap: 12, borderRadius: 14, backgroundColor: 0xFFFFFFFF, borderWidth: 1, borderColor: 0xFFE5E7EB);

        return Column::make(
            Text::make('Motion')->style(new Style(fontSize: 26, fontWeight: 700, textColor: 0xFF111827)),

            Column::make(
                Text::make('Implicit transition')->style($label),
                View::make()
                    ->style(new Style(
                        width: 56,
                        height: 56,
                        borderRadius: 16,
                        backgroundColor: $this->moved ? 0xFF7C3AED : 0xFF2563EB,
                        translationX: $this->moved ? 360.0 : 0.0,
                        rotation: $this->moved ? 180.0 : 0.0,
                        opacity: $this->moved ? 0.6 : 1.0,
                    ))
                    ->animate(600, AnimationEasing::EaseInOut),
            )->style($card),

            Row::make(
                Column::make(
                    Text::make('Spring')->style($label),
                    View::make()
                        ->style(new Style(width: 56, height: 56, borderRadius: 28, backgroundColor: 0xFFEF4444))
                        ->animation(
                            Animation::sequence(
                                Animation::set(['scale' => 0.4]),
                                Animation::spring(['scale' => 1.0], stiffness: 180.0, damping: 8.0),
                            )->key($this->bounces),
                        ),
                )->style(new Style(flexGrow: 1, gap: 12)),
                Column::make(
                    Text::make('Endless program')->style($label),
                    View::make()
                        ->style(new Style(width: 56, height: 56, borderRadius: 12, borderWidth: 6, borderColor: 0xFF10B981))
                        ->animation(Animation::timing(['rotate' => 360], 1400, Easing::Linear)->repeat()),
                )->style(new Style(flexGrow: 1, gap: 12)),
                Column::make(
                    Text::make('Preset')->style($label),
                    Text::make('Slide up')
                        ->style(new Style(fontSize: 18, fontWeight: 600, textColor: 0xFF111827, height: 56))
                        ->motion(MotionPreset::SlideUp, 500),
                )->style(new Style(flexGrow: 1, gap: 12)),
            )->style($card),

            Column::make(
                Text::make('Shimmer placeholders')->style($label),
                View::make()->style(new Style(height: 18, borderRadius: 9, backgroundColor: 0xFFE5E7EB, widthPercent: 70))
                    ->property(PropKey::ShimmerEnabled, true)
                    ->property(PropKey::ShimmerGradientColor, 0xCCFFFFFF),
                View::make()->style(new Style(height: 18, borderRadius: 9, backgroundColor: 0xFFE5E7EB))
                    ->property(PropKey::ShimmerEnabled, true)
                    ->property(PropKey::ShimmerGradientColor, 0xCCFFFFFF),
                View::make()->style(new Style(height: 18, borderRadius: 9, backgroundColor: 0xFFE5E7EB, widthPercent: 45))
                    ->property(PropKey::ShimmerEnabled, true)
                    ->property(PropKey::ShimmerGradientColor, 0xCCFFFFFF),
            )->style($card),

            Row::make(
                Button::make($this->moved ? 'Move back' : 'Move')->onPress(function (): void {
                    $this->moved = !$this->moved;
                })->style(new Style(flexGrow: 1)),
                Button::make('Bounce')->onPress(function (): void {
                    $this->bounces++;
                })->style(new Style(flexGrow: 1)),
            )->style(new Style(gap: 12)),
        )->style(new Style(padding: 24, gap: 14, flexGrow: 1, backgroundColor: 0xFFF3F4F6));
    }
}

App::run(new MotionDemo());
