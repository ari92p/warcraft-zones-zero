# Warcraft Zones Zero

A third-person action-combat prototype built on [benilla](https://github.com/samwhosung/benilla), its from-scratch World of Warcraft 1.12.1 client. The prototype starts in Scarlet Monastery and experiments with a three-character party, instant switching, melee combo strings, and a short-range special attack.

This is a separate game project layered on the benilla client. The client remains the source of truth for 1.12.1 world rendering, creature models, animation, and data loading. No World of Warcraft game data is included in this repository.

## Run

You need Rust and a local 1.12.1 client data install. To use OctoWoW, point benilla at its `Data` directory:

```sh
WOW_DATA="/path/to/OctoWoW/Data" cargo run -p dungeon-delver
```

The OctoWoW game executable is not launched: this prototype runs in benilla and reads the client's MPQ assets from `Data`. On a development checkout, benilla can also find a `WoW` link in the repository root as described in [`docs/METHOD.md`](docs/METHOD.md). The first startup may take a while while Rust compiles the Bevy client.

## Prototype controls

| Input | Action |
| --- | --- |
| Mouse movement | Orbit the action camera and aim |
| W / A / S / D | Move relative to the camera |
| Space | Jump |
| Tab or 1 / 2 / 3 | Switch active party member |
| Hold left mouse button | Repeating light attacks and combo chain |
| Hold right mouse button | Repeating heavy attacks |
| Q / E / R | Special attacks |

The current slice uses a fixed Scarlet Monastery spawn and a small group of local Scarlet enemies. Combat is local and offline; there is no server, dungeon progression, loot, boss encounter, party selection screen, or character-specific skill kit yet. Those systems will be built as the prototype grows.

## Project direction

The intended loop is a short third-person dungeon run: enter a wing, clear packs, swap characters to chain abilities, and defeat its boss. The first slice is deliberately focused on the feel of movement, camera framing, switching, and hit cadence before adding a full combat and progression model.

## Upstream

`benilla` is retained as a separate Git remote so upstream client work can be reviewed and integrated deliberately:

```sh
git remote -v
```

The default `origin` remote points to this project. Do not commit proprietary game data or extracted assets.
