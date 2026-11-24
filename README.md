# Sonora

Lecteur audio léger pour Windows : interface **Win32/GDI pure** (aucune bibliothèque graphique), décodage **Symphonia**
(mp3, wav, mp4/AAC, mkv/webm, ogg, vorbis), **libopus intégrée dans l'exe** (décodeur seul, aucune DLL), sortie **cpal/WASAPI**,
rééchantillonneur sinc maison. Aucun ffmpeg, aucun VLC.

## Compiler (Windows, Rust stable récent + outils C de Visual Studio)

    cargo build --release

Résultat : `target\release\sonora.exe`, un seul fichier (icône, manifeste et informations de version inclus, runtime C lié statiquement).
Les erreurs sont écrites dans `error_log.txt` à côté de l'exe.

## Tests

    cargo test
    # tests de décodage / moteur : SONORA_TESTDATA=<dossier de fichiers de test>
    # coût processeur : cargo test --release resampler_cost -- --ignored --nocapture

## Fonctions

- **Touches multimédia** (globales, même sans focus) : lecture/pause, lecture, suivant, précédent, arrêt, volume +, volume -, muet.
  Les commandes d'application (casques, télécommandes) lecture, pause, arrêt, suivant, précédent, volume sont aussi gérées.
  Volume +/- agissent par pas de 5 sur le volume de Sonora (`playback_commands.rs`, `media_keys.rs`).
- **Fenêtre redimensionnable** : la taille d'origine (900 x 520) est la taille maximale ; minimum 480 x 320. La disposition
  est responsive (`window_layout.rs`) : marges puis espacements se resserrent, calcul pur testé.
- **Icône dessinée en code** (`build.rs`) : plus aucun fichier image dans le projet ; images 16 à 256 px pour la barre de titre,
  la barre des tâches et l'Explorateur, choisies selon le DPI.
- **Dossiers système masqués** dans l'explorateur (`protected_folders.rs`) ; Program Files et Program Files (x86) restent visibles.

## Légèreté (mesurée)

- Exécutable d'environ 1,5 Mo (3,4 Mo avant optimisation).
- Flux audio ouvert à la première lecture, arrêté en pause, fermé après 10 s d'inactivité ; threads endormis (aucun minuteur).
- Mémoire de travail rendue à Windows à la réduction de la fenêtre et à la fermeture du flux inactif.
- Décodage : 0,1 à 0,2 % d'un cœur ; rééchantillonnage 44,1 -> 48 kHz stéréo : environ 1 % d'un cœur.

## Dossiers

| Fichier | Rôle |
|---|---|
| main.rs, main_window.rs, browser_window.rs | démarrage, fenêtre principale, fenêtre « File Explorer » |
| window_layout.rs | disposition responsive |
| win32_support.rs, win32_dialogs.rs, memory_trimming.rs | utilitaires GDI/DPI/icônes, boîtes de dialogue natives, nettoyage mémoire |
| player_state.rs, playback_commands.rs, media_keys.rs | état, commandes de lecture, touches multimédia |
| playlist_navigation.rs, auto_advance.rs, seek_controls.rs, volume_control.rs | navigation, passage automatique, ±10 s, volume |
| media_selection.rs, folder_browser.rs, protected_folders.rs | choix des médias, logique de l'explorateur, dossiers protégés |
| audio_engine.rs, media_decoder.rs, format_conversion.rs, sinc_resampler.rs | moteur, décodage, mixage des canaux, rééchantillonnage |
| opus_decoder.rs, libopus_bindings.rs, codec_registry.rs | Opus (libopus) branché sur Symphonia |

## libopus

`build.rs` ne compile que les 65 fichiers du **décodeur** (`vendor/libopus`, licence BSD dans `vendor/libopus/COPYING`).
L'encodeur (opus_encoder, celt_encoder, SILK encode/NSQ/VAD, analyse) n'est ni présent ni compilé.

## Symphonia allégé

`vendor/symphonia/*` : copies de symphonia-core, -metadata et -format-isomp4 0.5.5 sans les tables FFT de 8192 à 65536 points
(jamais utilisées : AAC 512, Vorbis 2048) et sans `encoding_rs` (étiquettes de métadonnées non lues). Lecture audio inchangée.
