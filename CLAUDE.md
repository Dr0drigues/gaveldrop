# gaveldrop

Moteur de tests où **un cas est un fichier YAML**. Un cas décrit comment invoquer
un programme, comment ses dépendances doivent répondre, et ce que le résultat
doit contenir.

## À lire avant toute modification

1. **`CONTRIBUTING.md`** — les règles de développement. Rythme TDD, les trois
   portes, la langue, l'interdiction de mettre de la logique dans le YAML.
2. **`ARCHITECTURE.md`** — la carte du code et les invariants. Ses encadrés
   « Invariant d'architecture » sont le contrat : les casser exige de modifier
   ce document dans le même commit, avec la raison.

Ne pas déduire l'architecture du code : le code n'existe pas encore, et quand il
existera il sera l'application de ces deux documents, pas leur source.

## Les trois portes

Avant tout commit, les trois passent — et la CI les revérifie sur chaque PR :

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

## Le flux git

**Jamais de commit direct sur `main`.** Une branche par tâche
(`<type>/<description-en-kebab>`), une PR, la CI verte avant fusion. Sur un projet
solo, la CI est le seul relecteur.

**Pas de squash à la fusion** : les corps de commit portent le *pourquoi*, et un
squash les détruit.

## Rappels qui coûtent cher à oublier

- **Identifiants et mots-clés en anglais. Prose, commentaires de documentation,
  messages d'erreur et noms de tests en français.**
- **Un nom de test décrit un comportement**, pas une fonction :
  `le_filet_repond_et_se_signale_dans_le_journal`, pas `test_catch_all`.
- **Pas de logique dans le format de cas.** Ni condition, ni boucle, ni
  interpolation. La logique part dans un exécutable, via un des trois
  branchements.
- **Aucun commentaire dans les fichiers.** Seuls `//!` et `///` sont autorisés —
  ni `//` dans un corps de fonction, ni `#` dans un fichier de configuration. Le
  « pourquoi » remonte : dans le `///` de l'élément, dans `reason = "…"` d'un
  attribut de lint, dans le message d'une assertion, dans le corps du commit, ou
  dans `ARCHITECTURE.md` / `CONTRIBUTING.md`. Quand une explication n'a aucun
  élément auquel s'attacher, extraire une fonction nommée dont le `///` la porte.
- **Pas de `unwrap()` ni de `expect()`** — c'est un lint `deny` du workspace, pas
  un vœu. Si un cas est vraiment impossible :
  `#[expect(clippy::unwrap_used, reason = "…")]` — `expect` plutôt que `allow`,
  parce qu'il avertit quand l'exemption devient inutile. Les tests en sont
  exemptés par un `cfg_attr` en tête de crate.
- **Tout élément public est documenté** (`missing_docs` + `-D warnings`).
- **Pas d'`async`, pas d'`unsafe`, pas de généricité spéculative.** Un paramètre
  de type ne s'introduit qu'avec un second implémenteur réel sous les yeux.
- **`thiserror` en bibliothèque, `anyhow` en binaire.** Le contexte va dans les
  champs de la variante, pas dans une chaîne formatée. Messages en minuscule,
  sans point final — ils s'enchaînent.
- **Unix seulement.** Aucun `#[cfg(windows)]`.
- **`gaveldrop-fake` ne dépend d'aucune autre crate du dépôt.** C'est le seul
  invariant que le compilateur ne rappellera pas.
- **Les commentaires de documentation du format de cas sont des infobulles
  utilisateur** : ils traversent le schéma JSON jusqu'à l'éditeur.

## Ce qui ne se commite pas

`.gitignore` les tient dehors, mais autant le savoir : `PROJECT-BRIEF.md` et
`docs/superpowers/`. On peut y écrire, ça ne sort pas du dépôt.
