//! Traits et types communs à tous les émulateurs d'emurust.
//!
//! Chaque cœur d'émulation (`gb-core`, etc.) implémente [`Emulator`].
//! Le launcher ne connaît que ce trait : il n'a pas besoin de savoir
//! comment chaque machine fonctionne à l'intérieur.

use std::fmt;

/// Bouton logique d'une manette, indépendant du clavier ou de la manette réelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Start,
    Select,
}

/// Description statique d'un système émulé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemInfo {
    /// Identifiant court et stable (ex. `"gb"`).
    pub id: &'static str,
    /// Nom lisible (ex. `"Game Boy"`).
    pub name: &'static str,
    /// Largeur de l'écran en pixels.
    pub screen_width: u32,
    /// Hauteur de l'écran en pixels.
    pub screen_height: u32,
    /// Images par seconde de la machine d'origine.
    pub frame_rate: f64,
    /// Extensions de fichiers acceptées, sans le point.
    pub extensions: &'static [&'static str],
}

/// Erreurs communes aux émulateurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmuError {
    /// Le fichier fourni n'est pas une ROM valide pour ce système.
    InvalidRom(String),
    /// La ROM utilise une fonctionnalité pas encore émulée.
    Unsupported(String),
    /// Le fichier de sauvegarde ne correspond pas à ce jeu.
    InvalidSave(String),
}

impl fmt::Display for EmuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmuError::InvalidRom(msg) => write!(f, "ROM invalide : {msg}"),
            EmuError::Unsupported(msg) => write!(f, "non supporté : {msg}"),
            EmuError::InvalidSave(msg) => write!(f, "sauvegarde invalide : {msg}"),
        }
    }
}

impl std::error::Error for EmuError {}

/// Interface que tout émulateur doit fournir au launcher.
pub trait Emulator {
    /// Informations sur le système émulé.
    fn info(&self) -> SystemInfo;

    /// Charge une ROM en mémoire et réinitialise la machine.
    fn load_rom(&mut self, data: &[u8]) -> Result<(), EmuError>;

    /// Remet la machine dans son état de démarrage.
    fn reset(&mut self);

    /// Exécute l'émulation jusqu'à la fin de l'image en cours.
    fn run_frame(&mut self);

    /// Image courante, un pixel par `u32` au format `0x00RRGGBB`,
    /// ligne par ligne, de taille `screen_width * screen_height`.
    fn framebuffer(&self) -> &[u32];

    /// Contenu à écrire dans le fichier de sauvegarde (`.sav`), ou `None`
    /// si le jeu n'a rien à sauvegarder (cartouche sans pile).
    fn save_data(&self) -> Option<Vec<u8>> {
        None
    }

    /// Recharge une sauvegarde produite par [`Emulator::save_data`].
    fn load_save_data(&mut self, _data: &[u8]) -> Result<(), EmuError> {
        Ok(())
    }

    /// Fréquence d'échantillonnage voulue pour le son, en Hz.
    fn set_sample_rate(&mut self, _rate: u32) {}

    /// Ajoute à `out` les échantillons produits depuis le dernier appel :
    /// stéréo entrelacé (gauche, droite), valeurs entre -1,0 et 1,0.
    /// Par défaut, un émulateur ne produit aucun son.
    fn drain_audio(&mut self, _out: &mut Vec<f32>) {}

    /// Indique si un bouton est appuyé ou relâché.
    fn set_button(&mut self, button: Button, pressed: bool);
}
