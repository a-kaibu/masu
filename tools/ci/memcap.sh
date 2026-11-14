#!/bin/bash
# Test runner under cargo mutants (CARGO_TARGET_*_RUNNER): a mutant that allocates without end
# (a loop bound turned around) fails at 3 GB of address space (beyond the guest's 2 GiB) instead of taking the machine, and
# one that writes without end fails at 2 GB a file (SIGXFSZ) instead of filling the runner's disk
# (#83: two runs ran out of room with 100 GB free at the start).
# glibc otherwise retains a large virtual arena per test/helper thread: the unmutated
# baseline hit RLIMIT_AS with only ~300 MB resident. Bound those reservations, not the
# test's coverage; the 3 GB address-space and 2 GB file limits remain unchanged.
export MALLOC_ARENA_MAX="${MALLOC_ARENA_MAX:-2}"
# (3 GB for the test itself, plus the 2 GiB guest address space sake's tests reserve at the
# start, MAP_NORESERVE, test_guest::enter: it counts as address space though it holds nothing;
# with 3 GB in all a 256 MB picture or a 512 MB mmio file found no room, #242)
ulimit -v 5100000
ulimit -f "${MEMCAP_FILE_BLOCKS:-4194304}"
exec "$@"
