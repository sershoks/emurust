# emurust

Suite d'émulateurs open source écrite entièrement en Rust, avec un launcher unique.

## Feuille de route

- [ ] Launcher (version ligne de commande en place)
- [ ] Game Boy (DMG)
- [ ] ... (les suivants viendront un par un)

## Légal

Ce projet ne contient et ne distribuera **aucune** ROM, BIOS, boot ROM ou autre
contenu protégé par le droit d'auteur. Tout le code est écrit à partir de zéro.
Pour tester, utilisez des ROMs homebrew libres ou des ROMs de test open source.

## Licence

emurust est distribué sous licence **GNU GPL v3.0**. Voir le fichier [LICENSE](LICENSE).
Toute version modifiée ou redistribuée doit rester open source sous la même licence.

## Structure

```
crates/
  emu-core/   traits communs (Emulator, Button, SystemInfo)
  gb-core/    cœur Game Boy
  launcher/   binaire `emurust`
```

## Compiler et lancer

```
cargo run
cargo test
```
