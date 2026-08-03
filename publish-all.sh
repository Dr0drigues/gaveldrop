#!/bin/sh
# Publishes the four crates to crates.io, in the only order that works.
#
#   gaveldrop-fake → gaveldrop → gaveldrop-cli, gaveldrop-conformance
#
# Each crate declares the one below it by version, so the registry has to already have that version
# before the next publish can resolve it. Putting gaveldrop first is what produced:
#
#   failed to select a version for the requirement `gaveldrop-fake = "^0.1.3"`
#   candidate versions found which didn't match: 0.1.2, 0.1.1, 0.1.0
#
# A version already on crates.io is skipped rather than treated as an error, so re-running after a
# failure part-way through is safe.
#
# The CI equivalent is the `crates` job in .github/workflows/release.yml, which runs on a version tag.
# This script is for publishing by hand — before CARGO_REGISTRY_TOKEN exists, or to finish a release
# off.
set -eu

AGENT="gaveldrop-publish (https://github.com/Dr0drigues/gaveldrop)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

if [ -z "${VERSION}" ]; then
    echo "no version found in Cargo.toml" >&2
    exit 1
fi

# Refuses an uncommitted tree. `--allow-dirty` was in the first version of this script and it is the
# wrong default: it uploads what is on disk, and a published version can be yanked but never replaced.
# Pass DIRTY=1 if you mean it.
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    if [ "${DIRTY:-}" = "1" ]; then
        echo "warning: publishing a modified tree, because DIRTY=1"
        DIRTY_FLAG="--allow-dirty"
    else
        echo "the tree has uncommitted changes, and a published version cannot be replaced." >&2
        echo "commit them, or re-run with DIRTY=1 if that is really what you want." >&2
        exit 1
    fi
else
    DIRTY_FLAG=""
fi

published() {
    code="$(curl -s -A "${AGENT}" -o /dev/null -w '%{http_code}' \
        "https://crates.io/api/v1/crates/$1/${VERSION}")"
    [ "${code}" = "200" ]
}

# Waits for the registry to serve what was just uploaded, so the next crate can resolve it. Polling
# with a deadline rather than a fixed sleep: too short and the next publish fails, too long and every
# release pays for it.
await() {
    deadline=$(( $(date +%s) + 180 ))
    while ! published "$1"; do
        if [ "$(date +%s)" -ge "${deadline}" ]; then
            echo "  $1 ${VERSION} did not appear on crates.io within three minutes" >&2
            exit 1
        fi
        sleep 5
    done
    echo "  available"
}

echo "publishing ${VERSION}"

for crate in gaveldrop-fake gaveldrop gaveldrop-cli gaveldrop-conformance; do
    if published "${crate}"; then
        echo "${crate}: already at ${VERSION}, skipping"
        continue
    fi

    echo "${crate}: publishing"
    # shellcheck disable=SC2086
    cargo publish -p "${crate}" --locked ${DIRTY_FLAG}
    await "${crate}"
done

echo "done: all four crates are at ${VERSION}"
