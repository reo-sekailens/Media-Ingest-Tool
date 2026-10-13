#!/bin/bash
# Runner-only tests on disposable images, never physical removable devices.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "macOS filesystem validation requires a native macOS runner" >&2
  exit 1
fi
task_root="$(mktemp -d "${TMPDIR:-/tmp}/mit-filesystems.XXXXXX")"
task_root="$(cd "$task_root" && pwd -P)"
task_mount="$task_root/mounted"
mkdir "$task_mount"
cleanup() {
  if /sbin/mount | /usr/bin/grep -Fq " on $task_mount ("; then
    /usr/bin/hdiutil detach "$task_mount"
  fi
}
trap cleanup EXIT
for filesystem in APFS APFSX ExFAT; do
  image="$task_root/$filesystem.sparseimage"
  if [[ "$filesystem" == APFSX ]]; then
    # hdiutil expects APFS; newfs_apfs -e selects case-sensitive names.
    # APFSX is a diskutil personality, not hdiutil's filesystem name.
    /usr/bin/hdiutil create -size 512m -type SPARSE -fs APFS -fsargs '-e' \
      -volname "MIT-$filesystem" "$image"
  else
    /usr/bin/hdiutil create -size 512m -type SPARSE -fs "$filesystem" \
      -volname "MIT-$filesystem" "$image"
  fi
  /usr/bin/hdiutil attach -nobrowse -mountpoint "$task_mount" "$image"
  mkdir "$task_mount/fixtures"
  # Verify the actual case policy instead of trusting the requested format name.
  printf 'case probe\n' > "$task_mount/fixtures/CaseProbe"
  if [[ "$filesystem" == APFSX ]]; then
    if [[ -e "$task_mount/fixtures/caseprobe" ]]; then
      echo 'Case-sensitive APFS image does not preserve distinct case' >&2
      exit 1
    fi
  elif [[ ! -e "$task_mount/fixtures/caseprobe" ]]; then
    echo "$filesystem image unexpectedly has case-sensitive names" >&2
    exit 1
  fi
  echo "Validating verified ingest on disposable $filesystem volume"
  # These execute the production byte-copy/readback/publication/receipt path.
  # Symlink-dependent tests belong to APFS; exFAT does not support symlinks.
  for test in \
    verified_copy_independently_hashes_and_commits \
    mac_publication_preserves_a_competing_file_and_the_verified_partial \
    mac_publication_refuses_existing_receipts_and_dangling_symlinks \
    inventory_excludes_mac_volume_metadata_but_keeps_nested_names_and_dotfiles \
    copy_accepts_nested_new_destinations_but_rejects_overlap_and_parent_traversal \
    cancelled_copy_keeps_an_entry_scoped_partial_until_explicit_recovery \
    recovery_rehashes_an_existing_final_file_and_rejects_mutation \
    receipt_is_written_as_versioned_json; do
    TMPDIR="$task_mount/fixtures" cargo test --locked --manifest-path src-tauri/Cargo.toml \
      "ingest::tests::$test" -- --exact --test-threads=1
  done
  /usr/bin/hdiutil detach "$task_mount"
done
echo "APFS, case-sensitive APFS, and exFAT real-file validation passed"
