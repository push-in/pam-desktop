<?php

declare(strict_types=1);

namespace RoutesApp\Views;

use Pam\Desktop\Component;

final class ChatCard extends Component
{
    public string $id = '';
}
?>

<template>
    <Link route="chat" :params="{ id: id, unread: 3 }"><Text>Open</Text></Link>
</template>
