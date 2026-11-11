<?php

declare(strict_types=1);

namespace App\Services;

/**
 * Colours source as VS Code's Dark Modern theme does, line by line. PHP goes
 * through PHP's own tokenizer; the markup of a `.pam.php` template, CSS, JSON
 * and Markdown through a few patterns. Each line is a list of [text, colour].
 */
final class Highlighter
{
    public const string TEXT = '#CCCCCC';
    public const string KEYWORD = '#569CD6';
    public const string CONTROL = '#C586C0';
    public const string STRING = '#CE9178';
    public const string COMMENT = '#6A9955';
    public const string VARIABLE = '#9CDCFE';
    public const string FUNCTION = '#DCDCAA';
    public const string TYPE = '#4EC9B0';
    public const string NUMBER = '#B5CEA8';
    public const string TAG = '#569CD6';
    public const string PUNCTUATION = '#808080';
    public const string CONSTANT = '#4FC1FF';

    private const array CONTROL_TOKENS = [
        T_IF, T_ELSE, T_ELSEIF, T_FOREACH, T_FOR, T_WHILE, T_DO, T_SWITCH, T_CASE, T_DEFAULT, T_BREAK,
        T_CONTINUE, T_RETURN, T_TRY, T_CATCH, T_FINALLY, T_THROW, T_MATCH, T_YIELD, T_REQUIRE, T_REQUIRE_ONCE,
        T_INCLUDE, T_INCLUDE_ONCE, T_USE, T_NAMESPACE,
    ];

    /** @return list<list<array{0: string, 1: string}>> */
    public static function lines(string $path, string $source): array
    {
        $runs = match (true) {
            str_ends_with($path, '.php') => self::php($source),
            str_ends_with($path, '.css') => self::patterns($source, self::CSS),
            str_ends_with($path, '.json') => self::patterns($source, self::JSON),
            str_ends_with($path, '.md') => self::patterns($source, self::MARKDOWN),
            default => [[$source, self::TEXT]],
        };

        return self::split($runs);
    }

    /** @return list<array{0: string, 1: string}> */
    private static function php(string $source): array
    {
        $runs = [];
        $tokens = token_get_all($source);
        foreach ($tokens as $index => $token) {
            if (is_string($token)) {
                $runs[] = [$token, self::TEXT];
                continue;
            }
            [$id, $text] = $token;
            if ($id === T_INLINE_HTML) {
                array_push($runs, ...self::patterns($text, self::MARKUP));
                continue;
            }
            $runs[] = [$text, self::phpColour($id, $text, $tokens, $index)];
        }

        return $runs;
    }

    /** @param list<mixed> $tokens */
    private static function phpColour(int $id, string $text, array $tokens, int $index): string
    {
        $next = self::significant($tokens, $index, 1);
        $previous = self::significant($tokens, $index, -1);

        return match (true) {
            $id === T_COMMENT || $id === T_DOC_COMMENT => self::COMMENT,
            $id === T_VARIABLE => self::VARIABLE,
            $id === T_CONSTANT_ENCAPSED_STRING || $id === T_ENCAPSED_AND_WHITESPACE || $id === T_START_HEREDOC || $id === T_END_HEREDOC => self::STRING,
            $id === T_LNUMBER || $id === T_DNUMBER => self::NUMBER,
            in_array($id, self::CONTROL_TOKENS, true) => self::CONTROL,
            $id === T_OPEN_TAG || $id === T_CLOSE_TAG => self::KEYWORD,
            $id === T_STRING && $next === '(' => self::FUNCTION,
            $id === T_STRING && in_array(strtolower($text), ['true', 'false', 'null'], true) => self::KEYWORD,
            $id === T_STRING && $previous === T_OBJECT_OPERATOR => self::VARIABLE,
            ($id === T_STRING || $id === T_NAME_QUALIFIED || $id === T_NAME_FULLY_QUALIFIED) && ctype_upper($text[0] ?? '') && $text !== strtoupper($text) => self::TYPE,
            $id === T_NAME_QUALIFIED || $id === T_NAME_FULLY_QUALIFIED => self::TYPE,
            $id === T_STRING && $text === strtoupper($text) && ctype_alpha($text[0] ?? '') => self::CONSTANT,
            in_array($id, [T_WHITESPACE, T_STRING, T_OBJECT_OPERATOR, T_NULLSAFE_OBJECT_OPERATOR, T_DOUBLE_COLON, T_DOUBLE_ARROW, T_COALESCE, T_IS_IDENTICAL, T_IS_NOT_IDENTICAL, T_ELLIPSIS], true) => self::TEXT,
            default => self::KEYWORD,
        };
    }

    /** @param list<mixed> $tokens */
    private static function significant(array $tokens, int $index, int $step): int|string|null
    {
        for ($at = $index + $step; isset($tokens[$at]); $at += $step) {
            $token = $tokens[$at];
            if (is_array($token) && $token[0] === T_WHITESPACE) {
                continue;
            }

            return is_array($token) ? $token[0] : $token;
        }

        return null;
    }

    private const array MARKUP = [
        '/<!--.*?-->/s' => self::COMMENT,
        '/\{\{.*?\}\}/s' => self::FUNCTION,
        '/<\/?[A-Za-z][\w.-]*|\/?>/' => self::TAG,
        '/[:@]?[A-Za-z][\w:.-]*(?==)/' => self::VARIABLE,
        '/"[^"]*"|\'[^\']*\'/' => self::STRING,
        '/[.#][A-Za-z][\w-]*(?=[^{};]*\{)/' => '#D7BA7D',
        '/[a-z-]+(?=\s*:[^:])/' => self::VARIABLE,
        '/#[0-9A-Fa-f]{3,8}\b|\b\d+(?:\.\d+)?(?:px|%|s|ms)?\b/' => self::NUMBER,
    ];

    private const array CSS = [
        '/\/\*.*?\*\//s' => self::COMMENT,
        '/@[\w-]+/' => self::CONTROL,
        '/"[^"]*"|\'[^\']*\'/' => self::STRING,
        '/[.#]?[A-Za-z][\w-]*(?=[^{};]*\{)|:root/' => '#D7BA7D',
        '/--?[a-z][\w-]*(?=\s*:)/' => self::VARIABLE,
        '/#[0-9A-Fa-f]{3,8}\b|\b\d+(?:\.\d+)?(?:px|%|s|ms|dp)?\b/' => self::NUMBER,
    ];

    private const array JSON = [
        '/"[^"]*"(?=\s*:)/' => self::VARIABLE,
        '/"[^"]*"/' => self::STRING,
        '/\b(?:true|false|null)\b/' => self::KEYWORD,
        '/-?\b\d+(?:\.\d+)?\b/' => self::NUMBER,
    ];

    private const array MARKDOWN = [
        '/^#{1,6} .*$/m' => self::KEYWORD,
        '/`[^`\n]+`/' => self::STRING,
        '/\*\*[^*\n]+\*\*/' => '#D7BA7D',
        '/\[[^\]\n]+\]\([^)\n]+\)/' => self::CONSTANT,
        '/^\s*(?:[-*]|\d+\.) /m' => self::CONTROL,
        '/^\|.*$/m' => self::TEXT,
    ];

    /**
     * The first pattern to match at each position wins; text between matches
     * keeps the default colour.
     *
     * @param array<string, string> $patterns
     * @return list<array{0: string, 1: string}>
     */
    private static function patterns(string $text, array $patterns): array
    {
        $combined = '/'.implode('|', array_map(
            static fn (string $pattern): string => '('.preg_replace('/^\/|\/[a-z]*$/', '', $pattern).')',
            array_keys($patterns),
        )).'/ms';
        $colours = array_values($patterns);
        $runs = [];
        $offset = 0;
        if (preg_match_all($combined, $text, $matches, PREG_OFFSET_CAPTURE | PREG_SET_ORDER) > 0) {
            foreach ($matches as $match) {
                [$whole, $at] = $match[0];
                if ($whole === '' || $at < $offset) {
                    continue;
                }
                if ($at > $offset) {
                    $runs[] = [substr($text, $offset, $at - $offset), self::TEXT];
                }
                $group = 1;
                while (!isset($match[$group]) || $match[$group][1] < 0 || $match[$group][0] === '') {
                    $group++;
                    if ($group > count($colours)) {
                        break;
                    }
                }
                $runs[] = [$whole, $colours[$group - 1] ?? self::TEXT];
                $offset = $at + strlen($whole);
            }
        }
        if ($offset < strlen($text)) {
            $runs[] = [substr($text, $offset), self::TEXT];
        }

        return $runs;
    }

    /**
     * @param list<array{0: string, 1: string}> $runs
     * @return list<list<array{0: string, 1: string}>>
     */
    private static function split(array $runs): array
    {
        $lines = [[]];
        foreach ($runs as [$text, $colour]) {
            $parts = explode("\n", str_replace("\t", '    ', $text));
            foreach ($parts as $index => $part) {
                if ($index > 0) {
                    $lines[] = [];
                }
                if ($part !== '') {
                    $line = &$lines[count($lines) - 1];
                    $last = array_key_last($line);
                    if ($last !== null && $line[$last][1] === $colour) {
                        $line[$last][0] .= $part;
                    } else {
                        $line[] = [$part, $colour];
                    }
                    unset($line);
                }
            }
        }

        return $lines;
    }
}
