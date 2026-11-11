<?php

declare(strict_types=1);

namespace Pam\Desktop\Code;

/**
 * Token rules per language: ordered `[pattern, type]` pairs tried at each
 * position, the first that matches wins (like Prism or Rouge). Patterns are
 * PCRE without delimiters; they may span lines (block comments, strings).
 *
 * @internal
 */
final class Grammar
{
    private const string NUMBER = '0[xX][0-9a-fA-F_]+|0[bB][01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?';

    /** @return list<array{0: string, 1: TokenType}>|null */
    public static function rules(string $language): ?array
    {
        $c = static fn (string $words): string => '\b(?:'.str_replace(' ', '|', $words).')\b';

        return match ($language) {
            'javascript', 'typescript' => [
                ['\/\/[^\n]*|\/\*[\s\S]*?\*\/', TokenType::Comment],
                ['`(?:\\\\[\s\S]|[^`\\\\])*`|"(?:\\\\.|[^"\\\\\n])*"|\'(?:\\\\.|[^\'\\\\\n])*\'', TokenType::String],
                [$c('if else for while do switch case default break continue return try catch finally throw await yield import export from as new delete'), TokenType::Control],
                [$c('const let var function class extends implements interface type enum async static get set of in instanceof typeof void this super public private protected readonly declare namespace abstract keyof'), TokenType::Keyword],
                [$c('true false null undefined NaN Infinity'), TokenType::Constant],
                [$c('string number boolean any unknown never object symbol bigint'), TokenType::Type],
                ['\b[A-Z][A-Za-z0-9_]*\b', TokenType::Type],
                ['[A-Za-z_$][\w$]*(?=\s*\()', TokenType::Function],
                [self::NUMBER, TokenType::Number],
                ['=>|[-+*\/%=!<>&|^~?:]+', TokenType::Operator],
                ['[{}()\[\];,.]', TokenType::Punctuation],
                ['[A-Za-z_$][\w$]*', TokenType::Variable],
            ],
            'json' => [
                ['"(?:\\\\.|[^"\\\\\n])*"(?=\s*:)', TokenType::Property],
                ['"(?:\\\\.|[^"\\\\\n])*"', TokenType::String],
                [$c('true false null'), TokenType::Constant],
                ['-?'.self::NUMBER, TokenType::Number],
                ['[{}\[\],:]', TokenType::Punctuation],
            ],
            'css' => [
                ['\/\*[\s\S]*?\*\/', TokenType::Comment],
                ['"(?:\\\\.|[^"\\\\\n])*"|\'(?:\\\\.|[^\'\\\\\n])*\'', TokenType::String],
                ['@[\w-]+', TokenType::Control],
                ['--[\w-]+|[\w-]+(?=\s*:(?!:))', TokenType::Property],
                ['#[0-9a-fA-F]{3,8}\b', TokenType::Number],
                ['-?\d*\.?\d+(?:px|em|rem|%|ms|s|vh|vw|fr|ch|deg)?\b', TokenType::Number],
                ['[.#][\w-]+|::?[\w-]+', TokenType::Type],
                ['[\w-]+(?=\()', TokenType::Function],
                ['\b(?:[a-z][\w-]*)\b(?=[^{}]*\{)', TokenType::Tag],
                ['[{}();:,>+~*]', TokenType::Punctuation],
            ],
            'html', 'xml', 'markup' => [
                ['<!--[\s\S]*?-->', TokenType::Comment],
                ['<\/?[\w:.-]+|\/?>', TokenType::Tag],
                ['\{\{[\s\S]*?\}\}', TokenType::Variable],
                ['"[^"]*"|\'[^\']*\'', TokenType::String],
                ['[:@]?[\w:.-]+(?==)', TokenType::Attribute],
                ['&[\w#]+;', TokenType::Constant],
                ['=', TokenType::Punctuation],
            ],
            'markdown' => [
                ['^#{1,6} [^\n]*', TokenType::Heading],
                ['^```[^\n]*\n[\s\S]*?^```', TokenType::String],
                ['`[^`\n]+`', TokenType::String],
                ['\*\*[^*\n]+\*\*|__[^_\n]+__|\*[^*\n]+\*|_[^_\n]+_', TokenType::Emphasis],
                ['!?\[[^\]\n]*\]\([^)\n]*\)|<https?:[^>\s]+>|https?:\/\/[^\s)]+', TokenType::Link],
                ['^\s*(?:[-*+]|\d+\.)\s', TokenType::Keyword],
                ['^>[^\n]*', TokenType::Comment],
            ],
            'sql' => [
                ['--[^\n]*|\/\*[\s\S]*?\*\/', TokenType::Comment],
                ['\'(?:\'\'|[^\'])*\'', TokenType::String],
                ['(?i)'.$c('select from where and or not insert into values update set delete create table index view drop alter add column primary key foreign references join left right inner outer on group by order having limit offset as distinct union all case when then else end begin commit rollback transaction if exists pragma default unique null is in like between returning with'), TokenType::Keyword],
                ['(?i)'.$c('integer int text real blob boolean varchar date datetime timestamp numeric'), TokenType::Type],
                [self::NUMBER, TokenType::Number],
                ['[A-Za-z_]\w*(?=\s*\()', TokenType::Function],
                ['[-+*\/%=<>!|]+', TokenType::Operator],
                ['[(),;.]', TokenType::Punctuation],
            ],
            'shell' => [
                ['#[^\n]*', TokenType::Comment],
                ['"(?:\\\\.|[^"\\\\])*"|\'[^\']*\'', TokenType::String],
                ['\$\{[^}]*\}|\$[\w@#?$!*-]', TokenType::Variable],
                [$c('if then else elif fi for while until do done case esac in function return exit break continue local export readonly set unset source'), TokenType::Control],
                ['^\s*[\w.\/-]+(?=\s|$)|(?<=\|\s|&&\s|;\s)[\w.\/-]+', TokenType::Function],
                ['--?[\w-]+', TokenType::Attribute],
                [self::NUMBER, TokenType::Number],
                ['[|&;<>()]+', TokenType::Operator],
            ],
            'python' => [
                ['#[^\n]*', TokenType::Comment],
                ['[rbfRBF]{0,2}(?:"""[\s\S]*?"""|\'\'\'[\s\S]*?\'\'\'|"(?:\\\\.|[^"\\\\\n])*"|\'(?:\\\\.|[^\'\\\\\n])*\')', TokenType::String],
                [$c('if elif else for while try except finally raise return yield import from as with pass break continue await async lambda'), TokenType::Control],
                [$c('def class global nonlocal del assert in is not and or'), TokenType::Keyword],
                [$c('True False None self'), TokenType::Constant],
                ['@[\w.]+', TokenType::Function],
                ['\b[A-Z][A-Za-z0-9_]*\b', TokenType::Type],
                ['[A-Za-z_]\w*(?=\s*\()', TokenType::Function],
                [self::NUMBER, TokenType::Number],
                ['[-+*\/%=!<>&|^~:]+', TokenType::Operator],
                ['[{}()\[\];,.]', TokenType::Punctuation],
                ['[A-Za-z_]\w*', TokenType::Variable],
            ],
            'rust' => [
                ['\/\/[^\n]*|\/\*[\s\S]*?\*\/', TokenType::Comment],
                ['b?"(?:\\\\.|[^"\\\\])*"|r#*"[\s\S]*?"#*|\'(?:\\\\.|[^\'\\\\])\'', TokenType::String],
                [$c('if else match for while loop break continue return in await yield'), TokenType::Control],
                [$c('fn let mut const static struct enum impl trait type pub use mod crate super self Self where as ref move unsafe async dyn extern'), TokenType::Keyword],
                [$c('true false None Some Ok Err'), TokenType::Constant],
                ['\'[a-z_]\w*\b(?!\')', TokenType::Keyword],
                ['#!?\[[^\]]*\]', TokenType::Attribute],
                ['\b(?:[A-Z][A-Za-z0-9_]*|i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|char|str)\b', TokenType::Type],
                ['[a-z_]\w*!', TokenType::Function],
                ['[a-z_]\w*(?=\s*(?:::<[^>]*>)?\()', TokenType::Function],
                [self::NUMBER, TokenType::Number],
                ['=>|->|::|[-+*\/%=!<>&|^~?]+', TokenType::Operator],
                ['[{}()\[\];,.:]', TokenType::Punctuation],
                ['[a-z_]\w*', TokenType::Variable],
            ],
            'go' => [
                ['\/\/[^\n]*|\/\*[\s\S]*?\*\/', TokenType::Comment],
                ['`[^`]*`|"(?:\\\\.|[^"\\\\\n])*"|\'(?:\\\\.|[^\'\\\\])\'', TokenType::String],
                [$c('if else for range switch case default break continue return go defer select fallthrough goto'), TokenType::Control],
                [$c('func var const type struct interface map chan package import'), TokenType::Keyword],
                [$c('true false nil iota'), TokenType::Constant],
                ['\b(?:[A-Z][A-Za-z0-9_]*|string|int|int64|int32|uint|uint64|float64|float32|bool|byte|rune|error|any)\b', TokenType::Type],
                ['[A-Za-z_]\w*(?=\s*\()', TokenType::Function],
                [self::NUMBER, TokenType::Number],
                [':=|[-+*\/%=!<>&|^]+', TokenType::Operator],
                ['[{}()\[\];,.:]', TokenType::Punctuation],
            ],
            'yaml' => [
                ['#[^\n]*', TokenType::Comment],
                ['^\s*[\w.-]+(?=\s*:)|^\s*-\s+[\w.-]+(?=\s*:)', TokenType::Property],
                ['"(?:\\\\.|[^"\\\\\n])*"|\'[^\'\n]*\'', TokenType::String],
                ['(?i)'.$c('true false null yes no on off'), TokenType::Constant],
                ['-?'.self::NUMBER, TokenType::Number],
                ['[:\-|>&*!]', TokenType::Punctuation],
            ],
            'toml', 'ini' => [
                ['[#;][^\n]*', TokenType::Comment],
                ['^\s*\[[^\]\n]+\]', TokenType::Type],
                ['^\s*[\w.-]+(?=\s*=)', TokenType::Property],
                ['"""[\s\S]*?"""|"(?:\\\\.|[^"\\\\\n])*"|\'[^\'\n]*\'', TokenType::String],
                [$c('true false'), TokenType::Constant],
                ['-?'.self::NUMBER, TokenType::Number],
                ['[=\[\]{},]', TokenType::Punctuation],
            ],
            'diff' => [
                ['^(?:\+\+\+|---)[^\n]*', TokenType::Heading],
                ['^@@[^\n]*', TokenType::Keyword],
                ['^\+[^\n]*', TokenType::Inserted],
                ['^-[^\n]*', TokenType::Deleted],
            ],
            'dockerfile' => [
                ['#[^\n]*', TokenType::Comment],
                ['(?im)^\s*(?:FROM|RUN|CMD|LABEL|EXPOSE|ENV|ADD|COPY|ENTRYPOINT|VOLUME|USER|WORKDIR|ARG|ONBUILD|STOPSIGNAL|HEALTHCHECK|SHELL)\b', TokenType::Keyword],
                ['"(?:\\\\.|[^"\\\\])*"|\'[^\']*\'', TokenType::String],
                ['\$\{?\w+\}?', TokenType::Variable],
                ['--[\w-]+', TokenType::Attribute],
            ],
            default => null,
        };
    }

    /** Language names and the file names and extensions they cover. */
    public const array EXTENSIONS = [
        'php' => ['php', 'phtml'],
        'javascript' => ['js', 'mjs', 'cjs', 'jsx'],
        'typescript' => ['ts', 'tsx', 'mts', 'cts'],
        'json' => ['json', 'jsonc', 'lock'],
        'css' => ['css', 'scss'],
        'html' => ['html', 'htm', 'vue', 'svelte'],
        'xml' => ['xml', 'svg', 'xsl', 'plist'],
        'markdown' => ['md', 'markdown'],
        'sql' => ['sql'],
        'shell' => ['sh', 'bash', 'zsh'],
        'python' => ['py', 'pyi'],
        'rust' => ['rs'],
        'go' => ['go'],
        'yaml' => ['yml', 'yaml'],
        'toml' => ['toml'],
        'ini' => ['ini', 'cfg', 'conf', 'env'],
        'diff' => ['diff', 'patch'],
    ];
}
