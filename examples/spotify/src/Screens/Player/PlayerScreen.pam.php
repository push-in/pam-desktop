<?php

declare(strict_types=1);

namespace App\Screens\Player;

use App\Services\Library;
use App\Services\Player;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class PlayerScreen extends Component
{
    public Library $library;

    public Player $player;

    /** home, playlist or search */
    #[State]
    public string $view = 'home';

    #[State]
    public ?string $playlistId = null;

    #[State(hotReload: false)]
    public string $query = '';

    /** Bumped when the player changes, so the views show what plays. */
    #[State]
    public int $revision = 0;

    public function mount(): void
    {
        $this->library = new Library();
        $this->player = Player::make($this->library);
    }

    public function home(): void
    {
        $this->view = 'home';
        $this->playlistId = null;
    }

    public function open(string $id): void
    {
        $this->view = 'playlist';
        $this->playlistId = $id;
    }

    public function play(string $id, ?string $trackId = null): void
    {
        $playlist = $this->library->playlist($id);
        if ($trackId === null && $this->player->contextId === $id) {
            $this->player->toggle();
        } else {
            $this->player->play($playlist, $trackId);
        }
        $this->revision++;
    }

    /** @param array{id: string, track?: string} $request */
    public function playFrom(array $request): void
    {
        $this->play($request['id'], $request['track'] ?? null);
    }

    public function changed(): void
    {
        $this->revision++;
    }

    public function search(): void
    {
        $this->view = 'search';
    }
}
?>

<template>
    <Column class="window">
        <Row class="topbar">
            <Row class="nav">
                <Icon name="ellipsis" font="Lucide" size="20" class="subdued" />
                <Icon name="chevron-left" font="Lucide" size="20" class="subdued" />
                <Icon name="chevron-right" font="Lucide" size="20" class="faint" />
            </Row>
            <Row class="center">
                <Pressable class="home" cursor="pointer" @press="home" tooltip="Home">
                    <Icon name="house" font="Lucide" size="22" :class="$view === 'home' ? 'white' : 'subdued'" />
                </Pressable>
                <Row class="search">
                    <Icon name="search" font="Lucide" size="22" class="subdued" />
                    <Input class="search-field" model="query" sync="immediate" placeholder="What do you want to play?"
                           placeholderColor="#B3B3B3" @focus="search" />
                    <View class="search-split" />
                    <Icon name="layout-grid" font="Lucide" size="20" class="subdued" />
                </Row>
            </Row>
            <Row class="account">
                <Row class="premium"><Text class="premium-text">Explore Premium</Text></Row>
                <Row class="install">
                    <Icon name="circle-arrow-down" font="Lucide" size="16" class="subdued" />
                    <Text class="install-text">Install App</Text>
                </Row>
                <Icon name="bell" font="Lucide" size="18" class="subdued" />
                <Icon name="users" font="Lucide" size="18" class="subdued" />
                <View class="avatar-ring">
                    <Text class="avatar">D</Text>
                </View>
            </Row>
        </Row>

        <Row class="body">
            <LibraryPanel :library="$library" :player="$player" :activeId="$playlistId" :revision="$revision"
                          @open="open" />
            <Column class="main">
                <SearchView p-if="$view === 'search'" :library="$library" :player="$player" :query="$query"
                            :revision="$revision" @open="open" @play="playFrom" />
                <HomeView p-else-if="$view === 'home'" :library="$library" :player="$player"
                          :revision="$revision" @open="open" @play="play" />
                <PlaylistView p-else-if="$view === 'playlist' && $playlistId !== null" :key="'playlist-' . $playlistId"
                              :library="$library" :player="$player" :playlistId="$playlistId" :revision="$revision"
                              @play="playFrom" @changed="changed" />
            </Column>
        </Row>

        <NowPlayingBar :player="$player" :revision="$revision" @changed="changed" />
    </Column>
</template>

<style scoped>
    .window { flex: 1; background-color: var(--base); }
    .topbar { height: 64px; padding: 0 8px 0 16px; align-items: center; justify-content: space-between; }
    .nav { width: 240px; gap: 16px; align-items: center; }
    .center { flex: 1; max-width: 546px; gap: 8px; align-items: center; }
    .home { width: 48px; height: 48px; border-radius: 24px; background-color: var(--elevated); align-items: center;
            justify-content: center; }
    .home:hover { transform: scale(1.04); background-color: var(--highlight); }
    .search { flex: 1; height: 48px; gap: 12px; padding: 0 16px 0 12px; align-items: center; border-radius: 24px;
              background-color: var(--elevated); border-width: 1px; border-color: #00000000; }
    .search:hover { background-color: var(--highlight); border-color: #FFFFFF33; }
    .search:focus-within { border-color: var(--text); }
    .search-field { flex: 1; height: 46px; border-width: 0; padding: 0; font-size: 16px; background-color: #00000000; }
    .search-split { width: 1px; height: 24px; background-color: var(--subdued); }
    .account { width: 360px; gap: 16px; align-items: center; justify-content: flex-end; }
    .premium { height: 32px; padding: 0 16px; align-items: center; border-radius: 16px; background-color: var(--text); }
    .premium:hover { transform: scale(1.04); }
    .premium-text { font-size: 14px; font-weight: 700; color: var(--base); }
    .install { gap: 6px; align-items: center; }
    .install-text { font-size: 14px; font-weight: 700; color: var(--subdued); }
    .avatar-ring { width: 48px; height: 48px; border-radius: 24px; background-color: var(--elevated); align-items: center;
                   justify-content: center; }
    .avatar { width: 32px; height: 32px; border-radius: 16px; background-color: #E8115B; font-size: 14px;
              font-weight: 700; color: var(--base); text-align: center; line-height: 32px; }
    .subdued { color: var(--subdued); }
    .faint { color: var(--faint); }
    .white { color: var(--text); }
    .body { flex: 1; gap: 8px; padding: 0 8px; min-height: 0; }
    .main { flex: 1; border-radius: 8px; background-color: var(--panel); overflow: hidden; }
</style>
