# emurust

Suite d'émulateurs open source écrite entièrement en Rust, avec un launcher unique.

## Feuille de route

- [ ] Launcher (fenêtre de jeu en place, menu à venir)
- [ ] Game Boy (DMG)
  - [x] CPU SM83 (tous les opcodes, timing au cycle près)
  - [x] Bus mémoire, timer, port série
  - [x] PPU (fond, fenêtre, sprites, interruptions STAT)
  - [x] Son (APU : 2 canaux carrés, onde, bruit, stéréo)
  - [x] Cartouches MBC1, MBC3 (avec horloge RTC) et MBC5
  - [ ] Sauvegardes (.sav)
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
cargo run --release -- chemin/vers/jeu.gb
cargo test
```

Touches : flèches = croix, X = A, Z = B, Entrée = Start, Retour arrière = Select, Échap = quitter.

## Tester le CPU avec une ROM de test

Les ROMs de test CPU open source écrivent leur résultat sur le port série.
Elles ne sont pas incluses dans ce dépôt : téléchargez-les vous-même depuis leur source.

```
cargo run --release -p gb-core --example serial -- chemin/vers/test.gb
```
