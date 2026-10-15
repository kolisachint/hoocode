/*
 * Test-only memory pressure for the load scenario (concurrency.md, phase 5).
 *
 * hoocode has no hook that allocates memory on request, so this shim adds one from
 * outside the product: loaded with LD_PRELOAD, it does nothing unless the process
 * is named `hoocode` and HOOCODE_PERF_ALLOC_MB is set. Then a background thread
 * waits for the trigger file, maps HOOCODE_PERF_ALLOC_MB MiB, touches every page
 * (so it is resident), writes the "allocated" marker, waits for the release file,
 * and unmaps. Programs the bash tool starts (sh, awk, ...) are not named `hoocode`,
 * so they are untouched; the memory check runs no subagents.
 *
 * Environment (set by scripts/perf/load_scenario.py --scenario memory):
 *   HOOCODE_PERF_ALLOC_MB       size in MiB
 *   HOOCODE_PERF_ALLOC_TRIGGER  path; the allocation starts when it exists
 *   HOOCODE_PERF_ALLOC_DONE     path; written with "allocated" then "released"
 *   HOOCODE_PERF_ALLOC_RELEASE  path; the mapping is freed when it exists
 *
 * Build: cc -shared -fPIC -O2 -pthread -o alloc_shim.so alloc_shim.c
 */
#define _GNU_SOURCE
#include <limits.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

static void mark(const char *path, const char *text) {
    if (!path) {
        return;
    }
    FILE *f = fopen(path, "w");
    if (f) {
        fputs(text, f);
        fclose(f);
    }
}

static void *worker(void *arg) {
    (void)arg;
    const char *mb = getenv("HOOCODE_PERF_ALLOC_MB");
    const char *trigger = getenv("HOOCODE_PERF_ALLOC_TRIGGER");
    const char *release = getenv("HOOCODE_PERF_ALLOC_RELEASE");
    const char *done = getenv("HOOCODE_PERF_ALLOC_DONE");
    struct stat st;
    size_t bytes = (size_t)strtoull(mb, NULL, 10) * 1024u * 1024u;
    if (!trigger || !release || bytes == 0) {
        return NULL;
    }
    while (stat(trigger, &st) != 0) {
        usleep(50000);
    }
    unsigned char *p = mmap(NULL, bytes, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) {
        mark(done, "failed");
        return NULL;
    }
    for (size_t i = 0; i < bytes; i += 4096) {
        p[i] = 1; /* touch each page so it counts as resident */
    }
    mark(done, "allocated");
    while (stat(release, &st) != 0) {
        usleep(50000);
    }
    munmap(p, bytes);
    mark(done, "released");
    return NULL;
}

__attribute__((constructor)) static void shim_start(void) {
    char exe[PATH_MAX];
    ssize_t n = readlink("/proc/self/exe", exe, sizeof(exe) - 1);
    if (n <= 0) {
        return;
    }
    exe[n] = '\0';
    const char *base = strrchr(exe, '/');
    base = base ? base + 1 : exe;
    if (strcmp(base, "hoocode") != 0 || !getenv("HOOCODE_PERF_ALLOC_MB")) {
        return;
    }
    /* Subagent children inherit the environment: only the top-level process allocates. */
    const char *depth = getenv("HOOCODE_SUBAGENT_DEPTH");
    if (depth && strcmp(depth, "0") != 0) {
        return;
    }
    pthread_t thread;
    if (pthread_create(&thread, NULL, worker, NULL) == 0) {
        pthread_detach(thread);
    }
}
