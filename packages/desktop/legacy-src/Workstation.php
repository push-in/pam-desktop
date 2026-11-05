<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Workstation
{
    /** @param array<string, mixed> $windows */
    private function __construct(
        public bool $singleInstance,
        public bool $forwardArguments,
        public bool $backgroundAgent,
        public bool $persistentServices,
        public ProcessIsolation $isolation,
        public int $processPoolSize,
        public bool $workspaceRestore,
        public int $autosaveMilliseconds,
        public bool $recoveryJournal,
        public int $idleSleepMilliseconds,
        public bool $startupSnapshot,
        public bool $lazyPlugins,
        public bool $treeShaking,
        public RenderBackend $renderBackend,
        public bool $dirtyRegions,
        public bool $accessibility,
        public bool $devtools,
        public bool $crashReports,
        public InstallerScope $installerScope,
        public ReleaseChannel $releaseChannel,
        public PerformanceBudget $performance,
        public array $windows,
    ) {
        if ($processPoolSize < 1 || $processPoolSize > 16) {
            throw new InvalidArgumentException('The workstation process pool must contain 1 to 16 workers.');
        }
        if ($autosaveMilliseconds < 250 || $autosaveMilliseconds > 3_600_000) {
            throw new InvalidArgumentException('Autosave must be between 250ms and one hour.');
        }
        if ($idleSleepMilliseconds < 100 || $idleSleepMilliseconds > 3_600_000) {
            throw new InvalidArgumentException('Idle sleep must be between 100ms and one hour.');
        }
        foreach ($windows as $id => $profile) {
            Identifier::assert($id, 'The workstation window identifier');
            if (!$profile instanceof WindowProfile) {
                throw new InvalidArgumentException('Every workstation window profile must be a WindowProfile.');
            }
        }
    }

    public static function defaults(): self
    {
        return new self(
            singleInstance: true,
            forwardArguments: true,
            backgroundAgent: false,
            persistentServices: false,
            isolation: ProcessIsolation::Shared,
            processPoolSize: 2,
            workspaceRestore: true,
            autosaveMilliseconds: 2_000,
            recoveryJournal: true,
            idleSleepMilliseconds: 30_000,
            startupSnapshot: true,
            lazyPlugins: true,
            treeShaking: true,
            renderBackend: RenderBackend::Automatic,
            dirtyRegions: true,
            accessibility: true,
            devtools: true,
            crashReports: true,
            installerScope: InstallerScope::CurrentUser,
            releaseChannel: ReleaseChannel::Stable,
            performance: new PerformanceBudget(),
            windows: ['main' => WindowProfile::primary()],
        );
    }

    public function processes(ProcessIsolation $isolation, int $poolSize = 2): self
    {
        return $this->copy(isolation: $isolation, processPoolSize: $poolSize);
    }

    public function instance(bool $single = true, bool $forwardArguments = true): self
    {
        return $this->copy(singleInstance: $single, forwardArguments: $forwardArguments);
    }

    public function runtime(
        int $idleSleepMilliseconds = 30_000,
        bool $startupSnapshot = true,
        bool $lazyPlugins = true,
        bool $treeShaking = true,
    ): self {
        return $this->copy(
            idleSleepMilliseconds: $idleSleepMilliseconds,
            startupSnapshot: $startupSnapshot,
            lazyPlugins: $lazyPlugins,
            treeShaking: $treeShaking,
        );
    }

    public function rendering(
        RenderBackend $backend = RenderBackend::Automatic,
        bool $dirtyRegions = true,
    ): self {
        return $this->copy(renderBackend: $backend, dirtyRegions: $dirtyRegions);
    }

    public function diagnostics(bool $devtools = true, bool $crashReports = true): self
    {
        return $this->copy(devtools: $devtools, crashReports: $crashReports);
    }

    public function agent(bool $background = true, bool $persistentServices = true): self
    {
        return $this->copy(backgroundAgent: $background, persistentServices: $persistentServices);
    }

    public function persistence(int $autosaveMilliseconds = 2_000): self
    {
        return $this->copy(
            workspaceRestore: true,
            autosaveMilliseconds: $autosaveMilliseconds,
            recoveryJournal: true,
        );
    }

    public function window(string $id, WindowProfile $profile): self
    {
        return $this->copy(windows: [...$this->windows, $id => $profile]);
    }

    public function release(ReleaseChannel $channel, InstallerScope $scope = InstallerScope::CurrentUser): self
    {
        return $this->copy(releaseChannel: $channel, installerScope: $scope);
    }

    public function performance(PerformanceBudget $budget): self
    {
        return $this->copy(performance: $budget);
    }

    /** @return array<string, mixed> */
    public function toArray(): array
    {
        $profiles = [];
        foreach ($this->windows as $id => $profile) {
            if (!$profile instanceof WindowProfile) {
                throw new InvalidArgumentException('Every workstation window profile must be a WindowProfile.');
            }
            $profiles[$id] = $profile->toArray();
        }

        return [
            'singleInstance' => $this->singleInstance,
            'forwardArguments' => $this->forwardArguments,
            'backgroundAgent' => $this->backgroundAgent,
            'persistentServices' => $this->persistentServices,
            'isolation' => $this->isolation->value,
            'processPoolSize' => $this->processPoolSize,
            'workspaceRestore' => $this->workspaceRestore,
            'autosaveMilliseconds' => $this->autosaveMilliseconds,
            'recoveryJournal' => $this->recoveryJournal,
            'idleSleepMilliseconds' => $this->idleSleepMilliseconds,
            'startupSnapshot' => $this->startupSnapshot,
            'lazyPlugins' => $this->lazyPlugins,
            'treeShaking' => $this->treeShaking,
            'renderBackend' => $this->renderBackend->value,
            'dirtyRegions' => $this->dirtyRegions,
            'accessibility' => $this->accessibility,
            'devtools' => $this->devtools,
            'crashReports' => $this->crashReports,
            'installerScope' => $this->installerScope->value,
            'releaseChannel' => $this->releaseChannel->value,
            'performance' => $this->performance->toArray(),
            'windows' => $profiles,
        ];
    }

    /** @param array<string, mixed>|null $windows */
    private function copy(
        ?bool $singleInstance = null,
        ?bool $forwardArguments = null,
        ?bool $backgroundAgent = null,
        ?bool $persistentServices = null,
        ?ProcessIsolation $isolation = null,
        ?int $processPoolSize = null,
        ?bool $workspaceRestore = null,
        ?int $autosaveMilliseconds = null,
        ?bool $recoveryJournal = null,
        ?int $idleSleepMilliseconds = null,
        ?bool $startupSnapshot = null,
        ?bool $lazyPlugins = null,
        ?bool $treeShaking = null,
        ?RenderBackend $renderBackend = null,
        ?bool $dirtyRegions = null,
        ?bool $devtools = null,
        ?bool $crashReports = null,
        ?InstallerScope $installerScope = null,
        ?ReleaseChannel $releaseChannel = null,
        ?PerformanceBudget $performance = null,
        ?array $windows = null,
    ): self {
        return new self(
            $singleInstance ?? $this->singleInstance,
            $forwardArguments ?? $this->forwardArguments,
            $backgroundAgent ?? $this->backgroundAgent,
            $persistentServices ?? $this->persistentServices,
            $isolation ?? $this->isolation,
            $processPoolSize ?? $this->processPoolSize,
            $workspaceRestore ?? $this->workspaceRestore,
            $autosaveMilliseconds ?? $this->autosaveMilliseconds,
            $recoveryJournal ?? $this->recoveryJournal,
            $idleSleepMilliseconds ?? $this->idleSleepMilliseconds,
            $startupSnapshot ?? $this->startupSnapshot,
            $lazyPlugins ?? $this->lazyPlugins,
            $treeShaking ?? $this->treeShaking,
            $renderBackend ?? $this->renderBackend,
            $dirtyRegions ?? $this->dirtyRegions,
            $this->accessibility,
            $devtools ?? $this->devtools,
            $crashReports ?? $this->crashReports,
            $installerScope ?? $this->installerScope,
            $releaseChannel ?? $this->releaseChannel,
            $performance ?? $this->performance,
            $windows ?? $this->windows,
        );
    }
}
