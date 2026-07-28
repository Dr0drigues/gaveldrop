# Règles de développement

Ce document est court à dessein : des règles qu'on ne lit pas ne sont pas des
règles. Il vaut pour tout le monde — personne humaine ou agent.

À lire avant de toucher au code : celui-ci, puis `ARCHITECTURE.md`.

## La règle qui commande les autres

**Les encadrés « Invariant d'architecture » de `ARCHITECTURE.md` sont le
contrat.** Une modification qui en casse un n'est pas un détail à corriger
discrètement. Deux issues seulement :

1. On renonce à la modification.
2. On modifie `ARCHITECTURE.md` **dans le même commit**, en écrivant la raison.

Il n'y a pas de troisième issue. Un invariant qui s'érode sans que personne
l'écrive est un invariant qui n'existait pas.

## Le rythme

Test d'abord, dans cet ordre, sans exception :

1. Écrire le test qui échoue.
2. **Le lancer, et vérifier qu'il échoue pour la bonne raison.**
3. Écrire l'implémentation minimale qui le fait passer.
4. Le relancer.
5. Commiter.

L'étape 2 n'est pas décorative. Un test qui passe avant qu'on ait écrit
l'implémentation ne teste rien, et on ne s'en aperçoit qu'à la première
régression qu'il aurait dû attraper. Vérifier *pourquoi* il échoue attrape aussi
le cas où il échoue à la compilation pour une faute de frappe.

Commits fréquents et petits. Un commit = un comportement.

## Les trois portes

Aucun commit ne franchit ces trois-là sans passer :

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

`-D warnings` n'est pas négociable. Un avertissement toléré une fois devient un
avertissement toléré toujours, et le bruit finit par cacher le signal.

## La langue

| En anglais | En français |
|---|---|
| identifiants, noms de types et de fonctions | prose des documents |
| mots-clés du format de cas | commentaires de documentation |
| préfixes de commit (`feat`, `fix`, `docs`…) | messages d'erreur |
| noms de crates | noms de tests |

Les mots-clés du format sont en anglais parce que le projet est destiné aux
registres publics. La prose est en français parce que son lecteur principal
l'est. Ce n'est pas une incohérence, c'est un arbitrage, et il est assumé.

## Conventions de code

### Ce qu'on n'utilise pas

**Pas d'`async`.** Le projet lance des processus et lit des fichiers. Une
exécution de suite est bornée par des appels système, pas par de l'attente
réseau. Tirer un runtime asynchrone serait du poids mort sur un binaire dont le
temps de démarrage est justement une contrainte de conception.

**Pas d'`unsafe`.** `forbid` au niveau du workspace, pas `deny` : la différence
est qu'on ne peut pas le lever localement.

**Pas de généricité spéculative.** Un paramètre de type se paie en bornes serde,
en bornes schemars, et en messages d'erreur du compilateur trois fois plus longs.
On ne l'introduit qu'avec un **second implémenteur réel** sous les yeux. `Rule`
était générique dans une première version du plan ; elle ne l'est plus, parce que
le seul consommateur imaginé déclare de toute façon son propre type.

**Pas de macro procédurale maison.** Les dérives des bibliothèques suffisent.

### Ce que la machine garantit

Deux règles ne dépendent pas de la bonne volonté. Elles sont dans les lints du
workspace, donc dans `-D warnings`, donc dans la CI :

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
```

**Pas de `unwrap()` ni de `expect()`.** Si un cas est réellement impossible, on le
lève localement — et la justification est **dans l'attribut**, pas à côté :

```rust
#[expect(
    clippy::unwrap_used,
    reason = "le filet garantit qu'une règle s'applique toujours ; \
              Scenario::load l'a vérifié"
)]
```

`expect` plutôt que `allow`, parce qu'il avertit aussi quand l'exemption devient
inutile — un `allow` oublié survit indéfiniment à la raison qui l'a fait naître.
Et « c'est évident » n'est pas une raison.

L'exemption des tests se déclare une fois par crate, pas une fois par module :

```rust
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "paniquer est le mode de signalement d'un test"
    )
)]
```

**Tout élément public est documenté** (`missing_docs`). Ce n'est pas de la
bureaucratie : pour les types du format de cas, ces documents **sont** les
infobulles vues dans l'éditeur, via le schéma JSON.

### Les erreurs

**`thiserror` en bibliothèque, `anyhow` en binaire.** Une bibliothèque rend des
erreurs que l'appelant peut discriminer ; un binaire les affiche et sort.

**Le contexte est dans les champs de la variante, pas dans une chaîne
formatée.** C'est ce qui permet à un appelant de réagir au chemin fautif plutôt
que d'en extraire le texte.

```rust
// Non — l'appelant ne peut rien faire de ça.
return Err(anyhow!("scénario invalide"));

// Oui — le chemin et la cause restent exploitables.
return Err(ScenarioError::Invalid { chemin: path.to_path_buf(), source });
```

**Les messages `#[error(...)]` commencent en minuscule et ne se terminent pas par
un point.** Ils s'enchaînent : `« scénario X illisible : ligne 4, colonne 2 »`. Un
point au milieu d'une chaîne d'erreurs la casse en deux phrases boiteuses.

**Un message d'erreur nomme le fautif** : le chemin, la clé, la valeur obtenue.
Et quand c'est possible, il dit **quoi faire** :

```rust
#[error(
    "scénario sans filet : ajouter une règle `match: {{}}` en dernier. \
     Sans elle, un appel imprévu se ferait passer pour un appel attendu."
)]
```

**Pas de `Box<dyn Error>`** dans une signature publique.

### Nommage

**Anglais, et sans abréviation dans ce qui est public.** Un type `Invocation`,
pas `Inv` ; une méthode `args_joined`, pas `args_j`. En revanche une variable
locale ou un paramètre peut être court quand sa portée tient dans un écran —
`inv`, `resp` — parce que la lisibilité y gagne plus qu'elle ne perd.

**Les fonctions sont des verbes à l'impératif** : `load`, `select`, `apply`,
`record`. Pas de préfixe `get_` sur un accesseur, c'est la convention Rust.

**Le style de module moderne** : `foo.rs` accompagné d'un dossier `foo/`, jamais
`foo/mod.rs`. Le nom du fichier suffit à savoir où on est, sans lire le chemin.

**`pub` est un engagement.** Ce qui n'est pas dans le contrat est `pub(crate)`.
Le contrat public est ré-exporté en tête de `lib.rs`, ce qui le rend lisible d'un
coup d'œil.

### Les commentaires

**Aucun commentaire dans les fichiers. Seuls les commentaires de documentation
sont autorisés** — `//!` en tête de module, `///` sur un élément. Pas de `//` dans
un corps de fonction, pas de `#` dans un fichier de configuration.

Le code se lit par sa clarté : par des noms justes, par des fonctions courtes, par
une décomposition qui rend l'intention visible. Un commentaire inline est une
rustine sur un défaut d'expression, et c'est une rustine qui vieillit sans que
personne s'en aperçoive — rien ne casse quand elle devient fausse.

**Donc le « pourquoi » remonte.** Il ne disparaît pas, il change d'adresse :

| Ce qu'on voulait dire | Où ça va maintenant |
|---|---|
| pourquoi ce module existe | son `//!` |
| pourquoi cet élément fait ça, quel piège il évite | son `///` |
| pourquoi cette exemption de lint | `reason = "…"` dans l'attribut |
| pourquoi ce test existe, contre quoi il protège | son nom, et le message de son assertion |
| pourquoi ce changement | le corps du message de commit |
| pourquoi cette structure | `ARCHITECTURE.md` |
| pourquoi cette configuration | ce document |

Le gain n'est pas seulement cosmétique : ce qui était enfoui dans un corps de
fonction devient de la documentation **publiée** par `rustdoc`, donc lue par qui
utilise la bibliothèque et pas seulement par qui la modifie.

**Et quand une explication n'a aucun élément auquel s'attacher**, c'est le signal
utile de cette règle : extraire une fonction nommée dont le `///` la portera. La
contrainte pousse vers la décomposition plutôt que vers l'annotation, et c'est
exactement le bon réflexe.

```rust
// Non — le commentaire porte ce que le code n'exprime pas.
// Sauter notre dossier, sinon le faux se rappellerait lui-même à l'infini.
let dirs = path.split(':').filter(|d| Some(*d) != skip);

// Oui — le nom et sa documentation portent la même chose, et rustdoc la publie.
/// Cherche `bin` dans `PATH`, en **sautant** `skip_dir`.
///
/// Sauter notre propre dossier est ce qui empêche le laisser-passer de se
/// rappeler lui-même à l'infini : le faux est en tête de `PATH` précisément sous
/// le nom du binaire qu'il remplace.
pub fn real_binary_in(bin: &str, path: &str, skip_dir: Option<&Path>) -> Option<PathBuf>
```

**Les exemples de documentation sont compilés** par `cargo test`. C'est la seule
sorte de documentation qui ne peut pas mentir — la préférer à une explication en
prose quand les deux sont possibles.

**Chaque module ouvre par un `//!` qui dit pourquoi il existe**, pas ce qu'il
contient — la liste des éléments, `rustdoc` la génère déjà.

### Formatage

`rustfmt` par défaut, **aucun `rustfmt.toml`**. Un fichier de configuration de
formatage est une invitation permanente à débattre de la largeur des lignes. S'il
faut vraiment dévier un jour, ce sera une décision écrite, pas un réglage glissé
au passage.

**Les commentaires de documentation des types du format de cas sont des
infobulles utilisateur.** Ils traversent le schéma JSON jusqu'à l'éditeur de qui
écrit un cas. Un champ mal documenté est un champ mal utilisé — ce n'est pas du
confort de lecture interne.

**Un fichier, une responsabilité.** Passé quelque quatre cents lignes, la
question n'est pas « faut-il découper » mais « qu'est-ce qui devrait sortir ».
Les choses qui changent ensemble restent ensemble ; on découpe par
responsabilité, pas par couche technique.

**Unix seulement.** Aucun `#[cfg(windows)]`, aucun contournement de portabilité.
Le code qui touche aux liens symboliques, aux permissions ou à `PATH` est sous
`#[cfg(unix)]`.

## Écrire des tests

**Le nom d'un test est une phrase qu'on lira dans un rapport d'échec.** Il décrit
le comportement attendu, pas la fonction visée.

```rust
// Non
fn test_catch_all() { … }

// Oui
fn le_filet_repond_et_se_signale_dans_le_journal() { … }
```

**Un test par comportement**, pas un test fourre-tout par fonction. Quand un
test échoue, son nom doit suffire à savoir ce qui est cassé.

**Quand un test protège contre un piège précis, le dire dans le message de
l'assertion.** C'est une chaîne, pas un commentaire — et c'est ce qu'on lira au
moment où le test échouera, c'est-à-dire au seul moment où ça compte :

```rust
assert_eq!(
    trouve, vrai_dir.join("git"),
    "sans sauter notre dossier, le faux se rappellerait lui-même à l'infini"
);
```

**Les trois niveaux ne se remplacent pas** — voir `ARCHITECTURE.md`,
« Préoccupations transverses » :

- **unitaires**, au plus près du code ;
- **le kit de conformité**, à la frontière de l'adaptateur — c'est la garantie
  propre de gaveldrop ;
- **les consommateurs réels**, hors de ce dépôt — c'est là que vit la preuve de
  non-régression, et elle n'est jamais rapatriée ici.

## Le format de cas

**Pas de logique dans le YAML.** Toute proposition qui ajoute au format une
condition, une boucle, une interpolation ou un calcul est refusée par défaut. Le
fichier de cas ne contient que des faits ; dès qu'il faut décider quelque chose,
ça part dans un exécutable via un branchement.

C'est la règle la plus facile à enfreindre avec de bonnes intentions, parce que
chaque ajout pris isolément paraît raisonnable. Le résultat cumulé est un langage
de programmation raté écrit en YAML.

**Le schéma commité est régénéré par un test**, jamais édité à la main. Si le
test de régénération échoue, c'est que le format a changé : on relance la
génération et on commite le schéma avec le changement de format, dans le même
commit.

**Trois branchements.** Un quatrième est une décision qu'on justifie dans
`ARCHITECTURE.md`, pas une dérive qu'on constate six mois plus tard.

## Les dépendances

**Une dépendance nouvelle se justifie dans le message de commit.** Pas dans une
conversation, pas dans une revue : dans l'historique, là où quelqu'un la
retrouvera en se demandant pourquoi elle est là.

**Les versions s'alignent sur celles du prototype** quand la dépendance y existe
— deux résolutions différentes dans un même arbre de compilation est un coût
qu'on ne paie pour rien.

**`gaveldrop-fake` ne dépend d'aucune autre crate du dépôt.** Invariant
d'architecture, et le seul que le compilateur ne rappellera pas de lui-même.

## Les commits

Préfixe conventionnel en anglais, sujet et corps en français :

```
feat(fake): compteur d'appels persistant, par clé

Chaque appel intercepté est un processus distinct, donc le compteur vit dans un
fichier. Le nom de fichier est assaini ET suffixé d'une empreinte, sinon « a/b »
et « a b » collisionneraient.
```

Le corps dit **pourquoi**, et surtout ce qui n'est pas devinable en lisant le
diff. Le piège évité, la solution écartée, la contrainte subie. Le *quoi* est
déjà dans le diff.

Portées : `fake`, `core`, `cli`, `conformance`, `docs`, `ci`.

## Le flux git

**Jamais de commit direct sur `main`.** Une branche, une PR, la CI verte avant
fusion. Y compris en solo — et surtout en solo : sur un projet à un seul
développeur, **la CI est le seul relecteur**. Se la court-circuiter revient à ne
plus être relu du tout.

Nommage de branche : `<type>/<description-en-kebab>`, avec le même vocabulaire de
type que les commits.

```
feat/compteur-appels
fix/passthrough-recursion
docs/regles-de-developpement
```

Une branche par tâche du plan. Le découpage en tâches a déjà le bon grain — il n'y
a pas de raison d'en inventer un second.

**Pas de squash à la fusion.** C'est une conséquence directe de la règle sur les
corps de commit : si chaque commit porte le *pourquoi* de son changement, écraser
la branche en un seul commit détruit exactement ce qu'on vient de demander
d'écrire. Rebase-and-merge, ou merge classique.

## Le socle

Les fichiers de configuration ne portent aucun commentaire — c'est cette section
qui porte leurs raisons. Quand on modifie l'un d'eux, on modifie aussi cette
section.

**Licence : `MIT OR Apache-2.0`**, le double standard de l'écosystème Rust. Elle
est au niveau du dépôt : pas d'en-tête de licence à recopier en tête des fichiers
source. Le texte de `LICENSE-APACHE` vient de `apache.org`, il n'a pas été
recopié de mémoire.

### `rust-toolchain.toml`

Chaîne **épinglée à 1.97**, avec `rustfmt` et `clippy` en composants pour que
l'intégration continue n'ait aucune étape d'installation.

Le plancher réel du code est **1.88** — les let-chains n'y sont stables qu'à
partir de là, et seulement en édition 2024. Il est déclaré par `rust-version` dans
le manifeste, ce qui donne un message clair au lieu d'une erreur de syntaxe
incompréhensible. On épingle plus haut pour être reproductible plutôt que juste
compatible.

Assumé : ce plancher n'est **pas** vérifié par l'intégration continue, qui tourne
sur la version épinglée. Monter la version épinglée est un commit d'une ligne — à
faire quand une raison l'exige, pas par réflexe à chaque sortie de Rust.

### `.github/workflows/ci.yml`

Les trois portes, vérifiées par une machine. Sur un projet solo, c'est le seul
relecteur.

`RUSTFLAGS: -D warnings` au niveau du workflow : un avertissement toléré une fois
devient un avertissement toléré toujours, et le bruit finit par cacher le signal.
C'est aussi ce qui transforme `missing_docs = "warn"` en refus effectif.

`concurrency` avec `cancel-in-progress` : une nouvelle poussée sur une branche
annule la vérification précédente de la même branche. C'est la dernière qui
compte, pas la file d'attente.

**Aucune action de chaîne d'outils.** `rust-toolchain.toml` est épinglé, et rustup
l'honore de lui-même au premier appel de cargo — une action ne ferait que
dupliquer la décision, avec le risque de la contredire. En revanche une étape
affiche les versions, pour qu'un échec dû à la chaîne reste distinguable d'un
échec dû au code.

**Deux plateformes, deux jobs asymétriques.** Le format et clippy ne tournent que
sur Linux : ils ne dépendent pas de la plateforme, et payer deux fois le même
résultat n'apporte rien. Les tests tournent sur Linux **et** macOS, parce que
l'isolation en dépend — liens symboliques, permissions, dossiers de
configuration — et que le morceau consacré au shell visera des chemins macOS.

### `Cargo.toml`

Les lints vivent au niveau du workspace, et chaque crate doit déclarer
`[lints] workspace = true` pour en hériter. Sans ce bloc, tout compile et rien
n'est vérifié : c'est un piège silencieux, et il n'existe aucun avertissement pour
le signaler.

Les versions de dépendances s'alignent sur celles du prototype quand la dépendance
y existe — deux résolutions différentes dans un même arbre de compilation est un
coût qu'on ne paie pour rien.

## Ce qui n'est pas une règle ici

Pas de seuil de couverture de tests. Une couverture chiffrée se satisfait en
écrivant des tests qui n'attrapent rien, et ce projet est précisément un outil
pour attraper des choses — ce serait une drôle de façon de commencer.
