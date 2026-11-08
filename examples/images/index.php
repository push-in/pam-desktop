<?php

declare(strict_types=1);

use Pam\Desktop\Align;
use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\ImageErrorEvent;
use Pam\Desktop\ImageFit;
use Pam\Desktop\ImageLoadEvent;
use Pam\Desktop\Justify;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Image;
use Pam\Desktop\UI\ImageBackground;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises images: remote photos through the disk cache, fits, a local
 * asset, a tinted template image, a data URI, a background image, a
 * placeholder and the load events.
 */
final class Images extends Component
{
    #[State]
    public string $status = 'Loading…';

    #[State]
    public int $loaded = 0;

    public function render(): Column
    {
        $label = new Style(fontSize: 12, textColor: 0xFF6B7485);
        $photo = static fn (int $id, int $width = 600, int $height = 400): string => "https://picsum.photos/id/{$id}/{$width}/{$height}";
        $tile = fn (string $caption, Image $image): Column => Column::make(
            $image->onLoad(function (ImageLoadEvent $event): void {
                $this->loaded++;
                $this->status = sprintf('%d loaded · last %d×%d', $this->loaded, $event->width, $event->height);
            })->onError(function (ImageErrorEvent $event): void {
                $this->status = 'Error: '.$event->message;
            }),
            Text::make($caption)->style($label),
        )->style(new Style(gap: 6));
        $box = new Style(width: 200, height: 130, borderRadius: 14, backgroundColor: 0xFFE8ECF3);

        return Column::make(
            Text::make('Images')->style(new Style(fontSize: 26, fontWeight: 700, textColor: 0xFF12161C)),
            Text::make($this->status)->style(new Style(fontSize: 13, textColor: 0xFF5B6472)),
            Row::make(
                $tile('cover (remote)', Image::make($photo(1015))->style($box)),
                $tile('contain', Image::make($photo(1025, 400, 600))->fit(ImageFit::Contain)->style($box)),
                $tile('fill', Image::make($photo(1035, 400, 400))->fit(ImageFit::Fill)->style($box)),
                $tile('placeholder + 404', Image::make('https://picsum.photos/id/999999/10/10')->defaultSource('asset://assets/placeholder.png')->style($box)),
            )->style(new Style(gap: 16)),
            Row::make(
                $tile('asset', Image::make('asset://assets/badge.png')->fit(ImageFit::Contain)->style(new Style(width: 96, height: 96, borderRadius: 20, backgroundColor: 0xFF12161C))),
                $tile('tinted', Image::make('asset://assets/star.png')->tint(0xFFFFB020)->fit(ImageFit::Contain)->style(new Style(width: 96, height: 96))),
                $tile('data URI', Image::make('data:image/svg+xml,'.rawurlencode('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96"><circle cx="48" cy="48" r="44" fill="#12B76A"/><path d="M28 50l14 14 28-30" stroke="#fff" stroke-width="9" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>'))->style(new Style(width: 96, height: 96))),
                $tile('repeat', Image::make('asset://assets/star.png')->fit(ImageFit::Repeat)->style(new Style(width: 200, height: 96, borderRadius: 12, backgroundColor: 0xFFFFF4D6))),
                $tile('circle + border', Image::make($photo(1027, 300, 300))->style(new Style(width: 96, height: 96, borderRadius: 48, borderWidth: 4, borderColor: 0xFF2F6BFF))),
            )->style(new Style(gap: 16, alignItems: Align::End)),
            ImageBackground::make(
                $photo(1043, 1200, 400),
                Text::make('ImageBackground')->style(new Style(fontSize: 22, fontWeight: 700, textColor: 0xFFFFFFFF)),
                Text::make('children are laid out above the picture')->style(new Style(fontSize: 13, textColor: 0xFFE8ECF3)),
            )->style(new Style(
                height: 150,
                borderRadius: 18,
                padding: 20,
                justifyContent: Justify::End,
                overflow: Pam\Desktop\Overflow::Hidden,
            )),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 16, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Images());
