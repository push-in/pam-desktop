<?php

declare(strict_types=1);

namespace Pam\Desktop\Code;

/**
 * Syntax highlighting: source to coloured runs, line by line.
 *
 * PHP goes through PHP's own tokenizer (exact), a `.pam.php` component also
 * colours its template; the other languages through ordered token rules.
 *
 * ```php
 * $language = Highlighter::language('src/App.php');           // 'php'
 * $lines = Highlighter::lines($source, $language);             // list<list<[text, ARGB]>>
 * Markdown::render($doc, highlight: Highlighter::runs(...));   // code blocks in Markdown
 * ```
 */
final class Highlighter
{
    /** @var array<string, array{0: string, 1: list<TokenType>}> combined pattern and group types, per language */
    private static array $compiled = [];

    private const array CONTROL = [
        T_IF, T_ELSE, T_ELSEIF, T_FOREACH, T_FOR, T_WHILE, T_DO, T_SWITCH, T_CASE, T_DEFAULT, T_BREAK,
        T_CONTINUE, T_RETURN, T_TRY, T_CATCH, T_FINALLY, T_THROW, T_MATCH, T_YIELD, T_YIELD_FROM, T_REQUIRE,
        T_REQUIRE_ONCE, T_INCLUDE, T_INCLUDE_ONCE, T_USE, T_NAMESPACE, T_ENDIF, T_ENDFOREACH, T_ENDWHILE, T_GOTO,
    ];

    private function __construct()
    {
    }

    /** The language of a file, by its name (`null` when unknown: shown as plain text). */
    public static function language(string $path): ?string
    {
        $name = strtolower(basename($path));
        if (str_ends_with($name, '.pam.php')) {
            return 'pam';
        }
        if ($name === 'dockerfile' || str_starts_with($name, 'dockerfile.')) {
            return 'dockerfile';
        }
        if (in_array($name, ['.gitignore', '.gitattributes', '.editorconfig', '.npmrc'], true)) {
            return 'ini';
        }
        $extension = pathinfo($name, PATHINFO_EXTENSION);
        foreach (Grammar::EXTENSIONS as $language => $extensions) {
            if (in_array($extension, $extensions, true)) {
                return $language;
            }
        }

        return null;
    }

    /** @return list<string> every language with rules */
    public static function languages(): array
    {
        return ['pam', ...array_keys(Grammar::EXTENSIONS), 'dockerfile'];
    }

    /** @return list<array{0: string, 1: TokenType}> the source cut into typed pieces (all of it, in order) */
    public static function tokens(string $source, ?string $language): array
    {
        $language = self::alias($language);

        return match (true) {
            $language === 'php' => self::php($source, false),
            $language === 'pam' => self::php($source, true),
            $language !== null && Grammar::rules($language) !== null => self::lex($source, $language),
            default => [[$source, TokenType::Text]],
        };
    }

    /**
     * @return list<array{0: string, 1: int}> coloured runs of the whole source,
     *         the shape `Markdown::render(highlight:)` takes
     */
    public static function runs(string $source, ?string $language, ?CodeTheme $theme = null): array
    {
        $theme ??= CodeTheme::darkModern();
        $runs = [];
        foreach (self::tokens($source, $language) as [$text, $type]) {
            $color = $theme->color($type);
            $last = array_key_last($runs);
            if ($last !== null && $runs[$last][1] === $color) {
                $runs[$last][0] .= $text;
            } else {
                $runs[] = [$text, $color];
            }
        }

        return $runs;
    }

    /** @return list<list<array{0: string, 1: int}>> one list of coloured runs per line, without the newlines */
    public static function lines(string $source, ?string $language, ?CodeTheme $theme = null): array
    {
        $lines = [[]];
        foreach (self::runs($source, $language, $theme) as [$text, $color]) {
            $parts = explode("\n", str_replace("\r\n", "\n", $text));
            foreach ($parts as $index => $part) {
                if ($index > 0) {
                    $lines[] = [];
                }
                if ($part !== '') {
                    $lines[array_key_last($lines)][] = [$part, $color];
                }
            }
        }

        return $lines;
    }

    private static function alias(?string $language): ?string
    {
        return match ($language === null ? null : strtolower($language)) {
            null, '' => null,
            'js', 'jsx', 'mjs' => 'javascript',
            'ts', 'tsx' => 'typescript',
            'sh', 'bash', 'zsh', 'console' => 'shell',
            'py' => 'python',
            'rs' => 'rust',
            'yml' => 'yaml',
            'md' => 'markdown',
            'htm', 'vue', 'svelte' => 'html',
            'svg' => 'xml',
            'patch' => 'diff',
            'docker' => 'dockerfile',
            default => strtolower($language),
        };
    }

    /** @return list<array{0: string, 1: TokenType}> */
    private static function lex(string $source, string $language): array
    {
        [$pattern, $types] = self::$compiled[$language] ??= self::compile($language);
        $tokens = [];
        $length = strlen($source);
        $position = 0;
        $plain = '';
        while ($position < $length) {
            if (preg_match($pattern, $source, $match, PREG_UNMATCHED_AS_NULL, $position) === 1 && $match[0] !== '') {
                foreach ($types as $index => $type) {
                    if (($match['g'.$index] ?? null) !== null) {
                        if ($plain !== '') {
                            $tokens[] = [$plain, TokenType::Text];
                            $plain = '';
                        }
                        $tokens[] = [$match[0], $type];
                        break;
                    }
                }
                $position += strlen($match[0]);
                continue;
            }
            // Nothing matched: a word, a run of spaces or one character is text.
            preg_match('/\G(?:\w+|\s+|.)/su', $source, $word, 0, $position);
            $plain .= $word[0] ?? $source[$position];
            $position += strlen($word[0] ?? $source[$position]);
        }
        if ($plain !== '') {
            $tokens[] = [$plain, TokenType::Text];
        }

        return $tokens;
    }

    /** @return array{0: string, 1: list<TokenType>} */
    private static function compile(string $language): array
    {
        $branches = [];
        $types = [];
        foreach (Grammar::rules($language) ?? [] as $index => [$rule, $type]) {
            $branches[] = "(?<g{$index}>{$rule})";
            $types[] = $type;
        }

        return ['/\G(?:'.implode('|', $branches).')/mu', $types];
    }

    /** @return list<array{0: string, 1: TokenType}> */
    private static function php(string $source, bool $template): array
    {
        $tokens = [];
        $all = token_get_all($source);
        foreach ($all as $index => $token) {
            if (is_string($token)) {
                $tokens[] = [$token, ctype_punct($token) && !in_array($token, ['(', ')', '[', ']', '{', '}', ';', ',', '.'], true) ? TokenType::Operator : TokenType::Punctuation];
                continue;
            }
            [$id, $text] = $token;
            if ($id === T_INLINE_HTML) {
                array_push($tokens, ...($template ? self::lex($text, 'markup') : [[$text, TokenType::Text]]));
                continue;
            }
            $tokens[] = [$text, self::phpType($id, $text, $all, $index)];
        }

        return $tokens;
    }

    /** @param list<mixed> $tokens */
    private static function phpType(int $id, string $text, array $tokens, int $index): TokenType
    {
        $next = self::significant($tokens, $index, 1);
        $previous = self::significant($tokens, $index, -1);

        return match (true) {
            $id === T_COMMENT || $id === T_DOC_COMMENT => TokenType::Comment,
            $id === T_VARIABLE => TokenType::Variable,
            in_array($id, [T_CONSTANT_ENCAPSED_STRING, T_ENCAPSED_AND_WHITESPACE, T_START_HEREDOC, T_END_HEREDOC], true) => TokenType::String,
            $id === T_LNUMBER || $id === T_DNUMBER => TokenType::Number,
            in_array($id, self::CONTROL, true) => TokenType::Control,
            $id === T_OPEN_TAG || $id === T_CLOSE_TAG || $id === T_OPEN_TAG_WITH_ECHO => TokenType::Keyword,
            $id === T_WHITESPACE => TokenType::Text,
            $id === T_ATTRIBUTE => TokenType::Punctuation,
            $id === T_STRING && in_array(strtolower($text), ['true', 'false', 'null'], true) => TokenType::Keyword,
            ($id === T_STRING || $id === T_NAME_QUALIFIED || $id === T_NAME_FULLY_QUALIFIED) && $next === '(' && $previous !== 'new' => TokenType::Function,
            $id === T_STRING && $previous === '->' || $id === T_STRING && $previous === '?->' => TokenType::Property,
            $id === T_STRING && $previous === '::' => ctype_upper(str_replace('_', '', $text)) ? TokenType::Constant : TokenType::Property,
            $id === T_STRING && ctype_upper(str_replace('_', '', $text)) && strlen($text) > 1 => TokenType::Constant,
            in_array($id, [T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED, T_NAME_RELATIVE, T_ARRAY, T_STATIC, T_CALLABLE], true) => TokenType::Type,
            in_array($id, [T_OBJECT_OPERATOR, T_NULLSAFE_OBJECT_OPERATOR, T_DOUBLE_COLON, T_DOUBLE_ARROW, T_IS_EQUAL, T_IS_IDENTICAL, T_IS_NOT_EQUAL, T_IS_NOT_IDENTICAL, T_COALESCE, T_BOOLEAN_AND, T_BOOLEAN_OR, T_INC, T_DEC, T_CONCAT_EQUAL, T_PLUS_EQUAL, T_MINUS_EQUAL, T_ELLIPSIS, T_SPACESHIP], true) => TokenType::Operator,
            default => TokenType::Keyword,
        };
    }

    /** The text of the nearest token that is not whitespace or a comment. @param list<mixed> $tokens */
    private static function significant(array $tokens, int $index, int $step): string
    {
        for ($i = $index + $step; isset($tokens[$i]); $i += $step) {
            $token = $tokens[$i];
            if (is_array($token) && in_array($token[0], [T_WHITESPACE, T_COMMENT, T_DOC_COMMENT], true)) {
                continue;
            }

            return is_string($token) ? $token : $token[1];
        }

        return '';
    }
}
