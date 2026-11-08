# PAM Desktop Printing

Native text documents and receipts. GTK uses its print dialog and Cairo/Pango, macOS uses AppKit's print panel and native text layout, and Windows uses the system print dialog and GDI. No browser or HTML renderer is involved.

```php
use Pam\DesktopPrinting\Printing;
use Pam\DesktopPrinting\PrintResult;
use Pam\DesktopPrinting\Receipt;

$document = (new Receipt('Receipt 42'))
    ->text('Payment receipt', bold: true, size: 12)
    ->text('Order: #42')
    ->separator()
    ->pair('2x Notebook', '$ 19.90')
    ->pair('Total:', '$ 19.90', bold: true)
    ->document();

Printing::print($document,
    function (PrintResult $result): void { /* Accepted or Cancelled. */ },
    function (Throwable $error): void { /* Display the printer error. */ },
);
```

The dialog includes the operating system's PDF output option where available. `Accepted` means the system accepted the job; it does not verify physical paper delivery. Cancellation is a separate result. One dialog/job may be active at a time. The app must keep the result callback alive until the system finishes; leaving a screen should suppress its stale UI updates, not misrepresent a queued print as cancelled.

`Document` contains uniform paper dimensions and pages of `TextBlock` objects, measured in points (72 points per inch). Blocks support Unicode, monospaced text, bold weight and integer `TextAlignment` enums. `Receipt` wraps using Unicode display widths, defaults to 80 mm paper and paginates bounded content. It can also be used for narrow thermal receipts without knowing a printer name.

Limits: 100 pages, 5,000 blocks, 1 MiB text and 2 MiB serialized document. Paper width is 72–2,880 points; height is 72–8,640 points. Blocks must fit their page. These limits are enforced in PHP and again in the native host. Windows spools on a worker thread so slow drivers do not block the app UI. Fonts/layout may vary slightly between operating systems.

Validation: `php packages/desktop-printing/tests/printing.php` from the framework repository, plus the core Rust printing tests. Linux native Print to File was exercised and its generated PDF's text and paper dimensions verified. AppKit and Win32 were cross-compiled; printer/device testing on those systems is still required.
