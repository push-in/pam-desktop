<?php

declare(strict_types=1);

namespace Pam\Desktop\Code;

/** What a piece of source is, for its colour. */
enum TokenType: string
{
    case Text = 'text';
    case Comment = 'comment';
    case String = 'string';
    case Number = 'number';
    case Keyword = 'keyword';
    case Control = 'control';
    case Type = 'type';
    case Function = 'function';
    case Variable = 'variable';
    case Constant = 'constant';
    case Operator = 'operator';
    case Punctuation = 'punctuation';
    case Tag = 'tag';
    case Attribute = 'attribute';
    case Property = 'property';
    case Heading = 'heading';
    case Emphasis = 'emphasis';
    case Link = 'link';
    case Inserted = 'inserted';
    case Deleted = 'deleted';
}
