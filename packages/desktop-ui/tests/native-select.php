<?php

declare(strict_types=1);

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopUi\\', dirname(__DIR__).'/src');
\Pam\Desktop\App::components(dirname(__DIR__).'/src');

final class NativeSelectFixture extends \Pam\Desktop\Component
{
    private \Pam\DesktopUi\Select $select;
    #[\Pam\Desktop\Attributes\State] public string $selected = 'Sem alteração';
    public function __construct()
    {
        $this->select = new \Pam\DesktopUi\Select([
            new \Pam\DesktopUi\SelectOption(1, 'Cozinha'),
            new \Pam\DesktopUi\SelectOption(2, 'Pedidos'),
            new \Pam\DesktopUi\SelectOption(3, 'Garçom'),
        ], 1, function (int|string|null $value): void {
            $this->select->value = $value;
            $this->selected = 'Selecionado: '.$value;
        }, label: 'Perfil', showClear: true, filter: true);
    }
    public function render(): \Pam\Desktop\Renderable
    {
        return \Pam\Desktop\UI\View::make(
            \Pam\Desktop\UI\Column::make(
                \Pam\Desktop\UI\Text::make('Select nativo reutilizável'),
                $this->select,
                \Pam\Desktop\UI\Text::make($this->selected),
            )->style(new \Pam\Desktop\Style(width: 340, padding: 32, gap: 16)),
            \Pam\Desktop\UI\Portal::host('desktop-ui'),
        )->nativeRef('desktop-ui')->style(new \Pam\Desktop\Style(flexGrow: 1, backgroundColor: 0xFFF8FAFC));
    }
}
\Pam\Desktop\App::run(new NativeSelectFixture());
