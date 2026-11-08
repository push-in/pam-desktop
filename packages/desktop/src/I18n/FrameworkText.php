<?php

declare(strict_types=1);

namespace Pam\Desktop\I18n;

use Pam\Desktop\Composables\Sources;

/**
 * Default copy of the framework (placeholders, accessibility labels, menu
 * roles, validation messages, native dialog titles): English as the source,
 * with Portuguese and Spanish included.
 *
 * The language is the app's I18n::use() override, else the system locale
 * (LC_ALL, LC_MESSAGES, LANG, then the locale the host reported), else
 * English. An app rewords or translates any text by registering the same key
 * under the `pam` namespace of its own catalog:
 *
 * ```php
 * I18n::catalog('fr', ['pam' => ['select.placeholder' => 'Choisir']]);
 * ```
 *
 * A value passed to a component (`placeholder:`, `label:`…) always wins.
 */
final class FrameworkText
{
    private const array CATALOG = [
        'en' => [
            'select.placeholder' => 'Select',
            'select.label' => 'Select',
            'select.search' => 'Search',
            'select.searchOption' => 'Search option',
            'select.empty' => 'No options found.',
            'select.clear' => 'Clear selection',
            'select.closeOptions' => 'Close options',
            'multiSelect.placeholder' => 'Select items',
            'multiSelect.label' => 'Select multiple items',
            'multiSelect.search' => 'Search items',
            'multiSelect.selected' => 'Selected',
            'autocomplete.label' => 'Search',
            'autocomplete.filter' => 'Filter {label}',
            'autocomplete.open' => 'Open {label} options',
            'autocomplete.close' => 'Close {label} options',
            'autocomplete.clear' => 'Clear {label}',
            'list.empty' => 'No items found.',
            'list.previous' => 'Previous',
            'list.next' => 'Next',
            'list.searchFailed' => 'Could not search. Try again.',
            'toast.close' => 'Close',
            'media.remove' => 'Remove image',
            'checkbox.label' => 'Checkbox',
            'datePicker.placeholder' => 'Select date',
            'calendar.months' => 'January|February|March|April|May|June|July|August|September|October|November|December',
            'calendar.weekdays' => 'Su|Mo|Tu|We|Th|Fr|Sa',
            'calendar.apply' => 'Apply',
            'calendar.cancel' => 'Cancel',
            'calendar.clear' => 'Clear',
            'calendar.hour' => 'Hour',
            'calendar.minute' => 'Minute',
            'calendar.dateFormat' => 'm/d/Y',
            'calendar.datePlaceholder' => 'mm/dd/yyyy',
            'files.saveTitle' => 'Save file',
            'chart.empty' => 'No data',
            'navigation.search' => 'Search',
            'form.required' => 'This field is required.',
            'form.email' => 'Enter a valid email address.',
            'form.matches' => 'The values do not match.',
            'form.minLength' => 'Use at least {length} characters.',
            'form.maxLength' => 'Use no more than {length} characters.',
            'async.offline' => 'You are offline.',
            'async.stale' => 'Showing saved content.',
            'menu.edit' => 'Edit',
            'menu.window' => 'Window',
            'menu.quit' => 'Quit',
            'menu.about' => 'About',
            'menu.preferences' => 'Preferences…',
            'menu.undo' => 'Undo',
            'menu.redo' => 'Redo',
            'menu.cut' => 'Cut',
            'menu.copy' => 'Copy',
            'menu.paste' => 'Paste',
            'menu.selectAll' => 'Select All',
            'menu.minimize' => 'Minimize',
            'menu.zoom' => 'Zoom',
            'menu.toggleFullScreen' => 'Toggle Full Screen',
            'menu.closeWindow' => 'Close Window',
            'menu.hide' => 'Hide',
            'menu.hideOthers' => 'Hide Others',
            'menu.showAll' => 'Show All',
            'menu.services' => 'Services',
            'menu.settings' => 'Settings…',
            'menu.enterFullScreen' => 'Enter Full Screen',
            'menu.close' => 'Close',
        ],
        'pt' => [
            'select.placeholder' => 'Selecionar',
            'select.label' => 'Selecionar',
            'select.search' => 'Buscar',
            'select.searchOption' => 'Buscar opção',
            'select.empty' => 'Nenhuma opção encontrada.',
            'select.clear' => 'Limpar seleção',
            'select.closeOptions' => 'Fechar opções',
            'multiSelect.placeholder' => 'Selecione os itens',
            'multiSelect.label' => 'Selecionar vários itens',
            'multiSelect.search' => 'Buscar itens',
            'multiSelect.selected' => 'Selecionado',
            'autocomplete.label' => 'Buscar',
            'autocomplete.filter' => 'Filtrar {label}',
            'autocomplete.open' => 'Abrir {label} opções',
            'autocomplete.close' => 'Fechar {label} opções',
            'autocomplete.clear' => 'Limpar {label}',
            'list.empty' => 'Nenhum item encontrado.',
            'list.previous' => 'Anterior',
            'list.next' => 'Próxima',
            'list.searchFailed' => 'Não foi possível buscar. Tente novamente.',
            'toast.close' => 'Fechar',
            'media.remove' => 'Remover imagem',
            'checkbox.label' => 'Caixa de seleção',
            'datePicker.placeholder' => 'Selecionar data',
            'calendar.months' => 'Janeiro|Fevereiro|Março|Abril|Maio|Junho|Julho|Agosto|Setembro|Outubro|Novembro|Dezembro',
            'calendar.weekdays' => 'Dom|Seg|Ter|Qua|Qui|Sex|Sáb',
            'calendar.apply' => 'Aplicar',
            'calendar.cancel' => 'Cancelar',
            'calendar.clear' => 'Limpar',
            'calendar.hour' => 'Hora',
            'calendar.minute' => 'Minuto',
            'calendar.dateFormat' => 'd/m/Y',
            'calendar.datePlaceholder' => 'dd/mm/aaaa',
            'files.saveTitle' => 'Salvar arquivo',
            'chart.empty' => 'Sem dados',
            'navigation.search' => 'Buscar',
            'form.required' => 'Este campo é obrigatório.',
            'form.email' => 'Informe um e-mail válido.',
            'form.matches' => 'Os valores não coincidem.',
            'form.minLength' => 'Use pelo menos {length} caracteres.',
            'form.maxLength' => 'Use no máximo {length} caracteres.',
            'async.offline' => 'Você está offline.',
            'async.stale' => 'Mostrando conteúdo salvo.',
            'menu.edit' => 'Editar',
            'menu.window' => 'Janela',
            'menu.quit' => 'Sair',
            'menu.about' => 'Sobre',
            'menu.preferences' => 'Preferências…',
            'menu.undo' => 'Desfazer',
            'menu.redo' => 'Refazer',
            'menu.cut' => 'Recortar',
            'menu.copy' => 'Copiar',
            'menu.paste' => 'Colar',
            'menu.selectAll' => 'Selecionar tudo',
            'menu.minimize' => 'Minimizar',
            'menu.zoom' => 'Zoom',
            'menu.toggleFullScreen' => 'Alternar tela cheia',
            'menu.closeWindow' => 'Fechar janela',
            'menu.hide' => 'Ocultar',
            'menu.hideOthers' => 'Ocultar outros',
            'menu.showAll' => 'Mostrar tudo',
            'menu.services' => 'Serviços',
            'menu.settings' => 'Ajustes…',
            'menu.enterFullScreen' => 'Entrar em tela cheia',
            'menu.close' => 'Fechar',
        ],
        'es' => [
            'select.placeholder' => 'Seleccionar',
            'select.label' => 'Seleccionar',
            'select.search' => 'Buscar',
            'select.searchOption' => 'Buscar opción',
            'select.empty' => 'No se encontraron opciones.',
            'select.clear' => 'Borrar selección',
            'select.closeOptions' => 'Cerrar opciones',
            'multiSelect.placeholder' => 'Seleccione los elementos',
            'multiSelect.label' => 'Seleccionar varios elementos',
            'multiSelect.search' => 'Buscar elementos',
            'multiSelect.selected' => 'Seleccionado',
            'autocomplete.label' => 'Buscar',
            'autocomplete.filter' => 'Filtrar {label}',
            'autocomplete.open' => 'Abrir opciones de {label}',
            'autocomplete.close' => 'Cerrar opciones de {label}',
            'autocomplete.clear' => 'Borrar {label}',
            'list.empty' => 'No se encontraron elementos.',
            'list.previous' => 'Anterior',
            'list.next' => 'Siguiente',
            'list.searchFailed' => 'No se pudo buscar. Inténtelo de nuevo.',
            'toast.close' => 'Cerrar',
            'media.remove' => 'Quitar imagen',
            'checkbox.label' => 'Casilla de verificación',
            'datePicker.placeholder' => 'Seleccionar fecha',
            'calendar.months' => 'Enero|Febrero|Marzo|Abril|Mayo|Junio|Julio|Agosto|Septiembre|Octubre|Noviembre|Diciembre',
            'calendar.weekdays' => 'Do|Lu|Ma|Mi|Ju|Vi|Sá',
            'calendar.apply' => 'Aplicar',
            'calendar.cancel' => 'Cancelar',
            'calendar.clear' => 'Borrar',
            'calendar.hour' => 'Hora',
            'calendar.minute' => 'Minuto',
            'calendar.dateFormat' => 'd/m/Y',
            'calendar.datePlaceholder' => 'dd/mm/aaaa',
            'files.saveTitle' => 'Guardar archivo',
            'chart.empty' => 'Sin datos',
            'navigation.search' => 'Buscar',
            'form.required' => 'Este campo es obligatorio.',
            'form.email' => 'Introduzca un correo electrónico válido.',
            'form.matches' => 'Los valores no coinciden.',
            'form.minLength' => 'Use al menos {length} caracteres.',
            'form.maxLength' => 'Use como máximo {length} caracteres.',
            'async.offline' => 'Está sin conexión.',
            'async.stale' => 'Mostrando contenido guardado.',
            'menu.edit' => 'Editar',
            'menu.window' => 'Ventana',
            'menu.quit' => 'Salir',
            'menu.about' => 'Acerca de',
            'menu.preferences' => 'Preferencias…',
            'menu.undo' => 'Deshacer',
            'menu.redo' => 'Rehacer',
            'menu.cut' => 'Cortar',
            'menu.copy' => 'Copiar',
            'menu.paste' => 'Pegar',
            'menu.selectAll' => 'Seleccionar todo',
            'menu.minimize' => 'Minimizar',
            'menu.zoom' => 'Zoom',
            'menu.toggleFullScreen' => 'Alternar pantalla completa',
            'menu.closeWindow' => 'Cerrar ventana',
            'menu.hide' => 'Ocultar',
            'menu.hideOthers' => 'Ocultar otros',
            'menu.showAll' => 'Mostrar todo',
            'menu.services' => 'Servicios',
            'menu.settings' => 'Ajustes…',
            'menu.enterFullScreen' => 'Entrar en pantalla completa',
            'menu.close' => 'Cerrar',
        ],
    ];

    private function __construct()
    {
    }

    /**
     * The text of `$key` in the framework language; `{name}` placeholders
     * are filled from `$params`.
     *
     * @param array<string, string|int|float> $params
     * @param ?string $locale a tag to use instead of the framework locale
     */
    public static function get(string $key, array $params = [], ?string $locale = null): string
    {
        $language = self::language($locale);
        $override = I18n::translateIn($language, 'pam.'.$key, $params);
        if ($override !== 'pam.'.$key) {
            return $override;
        }
        $text = self::CATALOG[$language][$key] ?? self::CATALOG['en'][$key] ?? $key;
        if ($params === []) {
            return $text;
        }
        $replace = [];
        foreach ($params as $name => $value) {
            $replace['{'.$name.'}'] = (string) $value;
        }

        return strtr($text, $replace);
    }

    /** Pipe-separated entries of `$key` (month names, weekday initials). @return list<string> */
    public static function list(string $key, ?string $locale = null): array
    {
        return explode('|', self::get($key, [], $locale));
    }

    /** The language of the framework copy (or of `$locale`): `en`, `pt`, `es`, or one an app catalog adds. */
    public static function language(?string $locale = null): string
    {
        $tag = $locale ?? self::locale();
        $language = I18n::language($tag);
        if (isset(self::CATALOG[$language]) || \in_array($language, I18n::locales(), true)) {
            return $language;
        }

        return 'en';
    }

    /**
     * The BCP 47 tag the framework copy follows: the I18n::use() override,
     * else the system locale, else `en`.
     */
    public static function locale(): string
    {
        $preferred = I18n::preferredLocale();
        if ($preferred !== null) {
            return $preferred;
        }
        foreach (['LC_ALL', 'LC_MESSAGES', 'LANG'] as $name) {
            $value = getenv($name);
            if (\is_string($value) && $value !== '' && $value !== 'C' && $value !== 'POSIX' && !str_starts_with($value, 'C.')) {
                return str_replace('_', '-', (string) preg_replace('/[.@].*$/', '', $value));
            }
        }
        $device = Sources::lastDevice()?->locale ?? '';

        return $device !== '' ? $device : 'en';
    }
}
