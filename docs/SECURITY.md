# Sécurité

Résumé de ce que MCManager protège et comment. Détails par fonction dans les autres documents.

## Comptes et connexion (mode serveur)
- Mots de passe dérivés (PBKDF2-SHA256, sel aléatoire), jamais stockés en clair ; limitation des tentatives par adresse ; sessions par cookie `HttpOnly`/`SameSite=Strict`.
- **Double authentification (TOTP, depuis la 1.1.2)** : *Utilisateurs → Double authentification*. Compatible avec toute application d'authentification
  (Aegis, 2FAS, Google Authenticator, Authy...) : saisissez la clé affichée (ou ouvrez le lien `otpauth://` sur le téléphone), confirmez avec un premier code,
  puis **notez les 8 codes de secours** (affichés une seule fois, usage unique).
  - Après le mot de passe, la connexion demande le code ; sans lui, aucune session n'est créée. Un code déjà utilisé est refusé (anti-rejeu) ; l'horloge peut dériver de ±30 s.
  - Le secret est **chiffré sur disque** avec la clé de l'instance ; changer son mot de passe ne désactive pas la double authentification.
  - Désactiver exige le mot de passe **et** un code (ou un code de secours). Un administrateur peut réinitialiser un compte (*Utilisateurs → 2FA ↺*) si le téléphone est perdu.
  - Chaque activation, désactivation, réinitialisation et échec de second facteur est inscrit au journal d'audit.
  - Le jeton d'API de la machine locale et le jumelage entre instances ont leur propre protection et ne sont pas concernés.
- **Droits par serveur** (lecture < console < contrôle < gestion, refus par défaut) et **journal d'audit** : voir [SERVER.md](SERVER.md).

## Mise à jour automatique
L'archive de la release est comparée à l'empreinte SHA-256 publiée dans `SHA256SUMS` **avant** de toucher à l'installation ; téléchargement limité à 200 Mo ;
retour arrière si le remplacement échoue en cours de route. Les installations par paquet (`.deb`, `.rpm`, Homebrew) et `MCManager.app` ne se mettent pas à jour seules.

## Imports et fichiers
Les extractions de zip refusent les chemins sortant du dossier (zip-slip) et sont plafonnées : 200 000 fichiers, 64 Go décompressés réellement écrits (protection contre les archives piégées).

## Assistant
Voir [ASSISTANT.md](ASSISTANT.md) : confirmation des écritures, `web_fetch` durci contre le SSRF, contenu des outils traité comme une donnée non fiable.

## Secrets
Clés d'API, token et webhook Discord, secrets TOTP : chiffrés sur disque, jamais renvoyés à l'interface, jamais écrits dans les journaux ni l'audit.

## Signaler une faille
Ouvrez un ticket privé (*Security advisories*) sur le dépôt GitHub plutôt qu'un ticket public.
