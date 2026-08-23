# Media Ingest Tool

Media Ingest Tool is a local-first desktop application for safely ingesting removable camera media. It uses Tauri 2, Rust, React, TypeScript, Vite, and Tailwind CSS.

The local foundation now includes conservative Windows removable-volume discovery, controlled SanDisk reader-slot calibration, bounded verified ingest, BLAKE3 destination readback, SQLite manifest/receipt sealing, camera/time destination sorting, and cooperative cancellation. These are local automated results, not a claim of hardware certification.

Physical immutable-card identity through every reader, hotplug/reconnect recovery, macOS/Linux adapters, a real quick-format provider, safe eject, and signed release artifacts remain incomplete or uncertified. The Format action is intentionally disabled until those exact-device safety gates exist.

## Development

Prerequisites and platform notes are in [Development guide](docs/development.md).

```powershell
npm install
npm run check
npm run tauri dev
```

The app has no broad frontend filesystem or shell access. Native functionality is exposed only through narrow typed Rust commands.

## AI-ready workflow

The repository includes an evidence-oriented memory bank and common AI-agent entry points:

- Start with [AGENTS.md](AGENTS.md) for the operating rules.
- Read [memory-bank/README.md](memory-bank/README.md) for the maintained project context.
- Create and track work through [memory-bank/tasks/_index.md](memory-bank/tasks/_index.md).
- Follow the researched [media-ingest delivery roadmap](memory-bank/tasks/ROADMAP.md).
- Record durable choices in [memory-bank/decisions/README.md](memory-bank/decisions/README.md).
- Use the GitHub templates for reproducible issues and pull requests.

Before implementation begins, complete the unknowns in `memory-bank/projectbrief.md` and `memory-bank/techContext.md`. Keep facts evidence-based: a build, local test, packaged artifact, and live deployment are distinct kinds of proof.

Validate the AI documentation at any time with:

```powershell
pwsh -NoProfile -File scripts/verify-memory-bank.ps1
```

## License

Licensed under [AGPL-3.0](LICENSE).
