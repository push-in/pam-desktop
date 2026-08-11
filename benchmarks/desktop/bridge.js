(() => {
    "use strict";

    const percentile = (sorted, ratio) => sorted[Math.min(
        sorted.length - 1,
        Math.floor(sorted.length * ratio),
    )];

    window.runPamBridgeBenchmark = async ({
        command = "benchmark.noop",
        warmup = 200,
        samples = 10_000,
    } = {}) => {
        if (!window.pam || !Number.isInteger(warmup) || !Number.isInteger(samples)) {
            throw new TypeError("The PAM bridge and integer sample counts are required.");
        }
        for (let index = 0; index < warmup; index += 1) {
            await pam.invoke(command);
        }
        const durations = [];
        const started = performance.now();
        for (let index = 0; index < samples; index += 1) {
            const callStarted = performance.now();
            await pam.invoke(command);
            durations.push((performance.now() - callStarted) * 1_000);
        }
        const elapsed = performance.now() - started;
        durations.sort((left, right) => left - right);
        return Object.freeze({
            command,
            samples,
            elapsedMilliseconds: elapsed,
            operationsPerSecond: samples / (elapsed / 1_000),
            microseconds: Object.freeze({
                p50: percentile(durations, 0.50),
                p95: percentile(durations, 0.95),
                p99: percentile(durations, 0.99),
            }),
            runtime: await pam.diagnostics.snapshot(),
        });
    };
})();
