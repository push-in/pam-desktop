<?php

declare(strict_types=1);

// An SSH terminal: connects with PAM_SSH_HOST, PAM_SSH_PORT, PAM_SSH_USER and
// PAM_SSH_PASSWORD (or PAM_SSH_KEY, a private key file), opens a shell and
// shows it in a native <Terminal>. Output and keystrokes do not pass through
// PHP; PHP only hears the title, the size and the end of the shell.

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Terminal;
use Pam\Desktop\UI\Text;
use Pam\DesktopSsh\Shell;
use Pam\DesktopSsh\Ssh;
use Pam\DesktopSsh\SshException;
use Pam\DesktopSsh\SshOptions;
use Pam\DesktopSsh\SshSession;

$loader = require __DIR__.'/../../vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopSsh\\', __DIR__.'/../../packages/ssh/src');
$loader->addPsr4('Pam\\Desktop\\', __DIR__.'/../../packages/desktop/src');

final class TerminalExample extends Component
{
    #[State]
    public string $channel = '';

    #[State]
    public string $status = 'Connecting…';

    private ?SshSession $session = null;

    public function mount(): void
    {
        $key = getenv('PAM_SSH_KEY') ?: '';
        $options = new SshOptions(
            getenv('PAM_SSH_HOST') ?: '127.0.0.1',
            getenv('PAM_SSH_USER') ?: 'pam',
            (int) (getenv('PAM_SSH_PORT') ?: 22),
            password: getenv('PAM_SSH_PASSWORD') ?: null,
            privateKey: $key !== '' ? (string) file_get_contents($key) : null,
        );
        $this->session = Ssh::connect(
            $options,
            function (SshSession $session): void {
                $this->status = 'Connected · '.$session->algorithm();
                $session->shell(function (Shell $shell): void {
                    $this->channel = $shell->channel();
                }, cols: 100, rows: 30);
            },
            function (SshException $error): void {
                $this->status = 'Failed: '.$error->getMessage();
            },
            owner: $this,
        );
    }

    public function render(): Column
    {
        $bar = Text::make($this->status)->style(new Style(fontSize: 12, textColor: 0xFFA0A8B8, padding: 6));
        if ($this->channel === '') {
            return Column::make($bar)->style(new Style(flexGrow: 1, backgroundColor: 0xFF1B1D23));
        }

        return Column::make(
            $bar,
            Terminal::make($this->channel)
                ->onTitle(function (string $title): void {
                    $this->status = $title === '' ? 'Connected' : $title;
                })
                ->onResize(function (int $cols, int $rows): void {
                    $this->status = "{$cols}×{$rows}";
                })
                ->onExit(function (int $code): void {
                    $this->status = "Shell exited ({$code})";
                    $this->channel = '';
                })
                ->style(new Style(flexGrow: 1, padding: 6)),
        )->style(new Style(flexGrow: 1, backgroundColor: 0xFF1B1D23));
    }
}

App::run(new TerminalExample());
