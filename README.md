# emurust

Suite d'émulateurs open source écrite entièrement en Rust, avec un launcher unique.

## Feuille de route

- [x] Launcher néon rétro : carrousel des consoles, liste des jeux avec fiche et aperçu en direct, effet CRT, navigation au clavier
- [ ] Game Boy (DMG)
  - [x] CPU SM83 (tous les opcodes, timing au cycle près)
  - [x] Bus mémoire, timer, port série
  - [x] PPU (fond, fenêtre, sprites, interruptions STAT)
  - [x] Son (APU : 2 canaux carrés, onde, bruit, stéréo)
  - [x] Cartouches MBC1, MBC3 (avec horloge RTC) et MBC5
  - [x] Sauvegardes (.sav compatibles avec les autres émulateurs, horloge MBC3 incluse)
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
cargo run --release
```

Au premier lancement, le launcher crée un dossier `roms` avec un sous-dossier par console :

```
roms/
  gb/      vos jeux Game Boy (.gb)
```

Placez-y vos propres ROMs, cliquez sur **Actualiser**, puis sur un jeu pour le lancer.
Les sauvegardes (`.sav`) sont écrites à côté de chaque ROM.

Options :

```
emurust jeu.gb          lance directement un jeu
emurust --roms <dir>    utilise un autre dossier de ROMs
```

Dans le launcher : flèches pour choisir, Entrée pour valider, Échap pour revenir,
une lettre pour sauter aux jeux qui commencent par elle, F5 pour actualiser,
F3 pour ouvrir le dossier des ROMs, F2 pour l'effet écran cathodique.

En jeu : flèches = croix, X = A, Z = B, Entrée = Start, Retour arrière = Select,
Échap = retour à la liste des jeux.

Manettes (Xbox, PlayStation, génériques…) : croix ou stick gauche pour se déplacer,
A pour valider, B pour revenir, LB / RB pour changer de page. En jeu, les boutons
A, B, Start et Select correspondent à ceux de la console, et Select + Start
revient à la bibliothèque.

## Tester le CPU avec une ROM de test

Les ROMs de test CPU open source écrivent leur résultat sur le port série.
Elles ne sont pas incluses dans ce dépôt : téléchargez-les vous-même depuis leur source.

```
cargo run --release -p gb-core --example serial -- chemin/vers/test.gb
```
