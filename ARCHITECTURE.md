# Architecture de gaveldrop

> **Statut :** cadrage validé le 28 juillet 2026. Aucun code écrit.
> Ce document décrit l'architecture **cible**. Il se lit seul : c'est ici que
> vivent les décisions et les invariants, et rien d'autre n'a besoin d'être
> ouvert pour les comprendre.

Le format de cas a d'abord existé, et fait ses preuves, dans un prototype soudé
à un seul projet : **armadai**, un orchestrateur d'agents en Rust — neuf cas,
quinze cents lignes de harnais. gaveldrop en est l'extraction et la
généralisation, et armadai en est le premier consommateur. Les renvois au
« prototype » et à « armadai » dans ce document désignent ce code-là.

Ce document s'adresse à quelqu'un qui va modifier gaveldrop. Il dit **où sont
les choses et pourquoi elles y sont**, pas comment s'appellent les fonctions.

## Vue d'ensemble

gaveldrop exécute des tests dont **un cas est un fichier YAML**. Un cas décrit
comment invoquer un programme, comment ses dépendances doivent répondre, et ce
que le résultat doit contenir. gaveldrop prépare un environnement isolé,
invoque, observe, puis rend un verdict.

Trois propriétés commandent toutes les décisions qui suivent, dans cet ordre :

1. **Un cas est lisible et écrivable à la main** — donc générable par un agent.
   C'est ce qui rend la couverture bon marché, et c'est ce qui se dégrade en
   premier quand on relâche l'attention.
2. **Le projet testé n'a rien à changer** pour devenir testable. Ni code
   d'instrumentation, ni mode test dans le code de production.
3. **Un échec est diagnosticable sans lire gaveldrop.** Le rapport dit quel
   cas, quelle attente, quelle valeur obtenue.

**Invariant d'architecture :** le noyau ne connaît ni langage, ni framework, ni
outil. Il connaît des processus, des fichiers et des lignes de texte. Toute
connaissance d'une techno particulière vit dans un adaptateur ou dans un
exécutable fourni par le projet.

**Invariant d'architecture :** le fichier de cas ne contient que des **faits** —
un chemin, une chaîne attendue, un code de sortie, un contenu de fichier. Dès
qu'il faut *décider* quelque chose, la logique sort du YAML et part dans un
exécutable. C'est la digue contre la dérive vers un langage de programmation
raté écrit en YAML. Un YAML qui gagne des conditions et des boucles vieillit
mal ; un YAML qui appelle un script vieillit comme le script.

**Invariant d'architecture :** tout ce qui traverse une frontière d'extension
est de la donnée sérialisable en JSON. Jamais une poignée de fichier, jamais une
fonction, jamais un objet vivant. On n'exploite pas cette propriété aujourd'hui —
les extensions sont des exécutables, et un consommateur Rust passe par la
bibliothèque — mais on ne se l'interdit pas.

**Invariant d'architecture :** Unix seulement. L'isolation repose sur les liens
symboliques, sur `PATH` et sur les dossiers de configuration à la mode Unix.
Windows n'est pas un ajustement, c'est un autre projet.

### La règle de placement

Sans règle explicite, la frontière entre le noyau et les extensions devient une
négociation à chaque nouvelle observation. Elle est donc fixée une fois :

> **Le noyau porte tout ce qui est observable de n'importe quel processus** —
> code de sortie, sorties standard et d'erreur, fichiers écrits, appels
> sortants.
> **Une extension est réservée à ce que la techno seule peut produire** —
> typiquement des métriques internes ou un état que rien d'extérieur ne voit.

Conséquence directe, et elle a déjà été tranchée : les « événements » d'un
programme qui émet des lignes JSON sur sa sortie standard sont **observables de
n'importe quel processus**. Ils sont donc dans le noyau, pas dans une extension.

## Code Map

```
crates/
├── gaveldrop-fake/          le moteur de faux : bibliothèque + programme
├── gaveldrop/               le noyau : case, iso, adapters, verdict, report
├── gaveldrop-cli/           la façade en ligne de commande
└── gaveldrop-conformance/   le kit de conformité
```

Le sens des dépendances est `gaveldrop-cli → gaveldrop → gaveldrop-fake`. La
crate la plus réutilisable est en bas, et c'est voulu.

### `crates/gaveldrop-fake`

Le moteur de faux : décider quelle règle s'applique à un appel, tenir un
compteur, journaliser. C'est **à la fois une bibliothèque et un programme**.

Le programme est un `main()` d'une trentaine de lignes au-dessus de la
bibliothèque. Il est déposé en lien symbolique sous chacun des noms à simuler
(`git`, `kubectl`, `claude`…) et placé en tête de `PATH`.

**Frontière d'API.** Un projet Rust qui a besoin d'un habillage particulier
compile *son* faux binaire à partir de cette bibliothèque, avec son propre
rendu. C'est le chemin que prend armadai pour émettre le format de fil de
Claude Code.

**Invariant d'architecture :** `gaveldrop-fake` ne dépend d'aucune autre crate
du dépôt. Si elle finit par dépendre du noyau, un consommateur qui ne veut que
le moteur se retrouve à tirer l'évaluation, les rapports et le format de cas.

**Invariant d'architecture :** un scénario sans filet — une règle dont le
`match` est vide — est une **erreur de chargement**, pas un défaut toléré. Le
filet est ce qui transforme « un appel imprévu a eu lieu » en échec bruyant au
lieu d'un silence. C'est la propriété qui fait qu'un cas prouve quelque chose.

**Invariant d'architecture :** journaliser est inconditionnel. Un appel est
journalisé même quand la règle laisse passer vers le vrai binaire, même quand
c'est le filet qui a répondu, même quand le faux sort en erreur. Le journal est
la seule source de vérité sur *qui a appelé quoi*, et un journal à trous est
pire qu'un journal absent.

**Invariant d'architecture :** le journal est un **fichier en ajout**, jamais un
tuyau ni un socket. Chaque invocation du faux y ajoute une ligne ; le noyau le
relit après coup. C'est ce qui fait que le mécanisme marche sans synchronisation
quand le sujet lance des processus en parallèle.

**Invariant d'architecture :** la clé du compteur d'appels est fournie par
l'appelant, pas déduite. Par défaut c'est le nom du binaire simulé ; armadai y
met un identifiant d'agent extrait du prompt. Deux sémantiques différentes, un
seul mécanisme, et le choix est visible dans le code du programme plutôt que
caché dans le moteur.

Critères de `match` du noyau : `bin`, `args_contain`, `stdin_contains`,
`call: N`, et le filet (`match: {}`). Un projet en ajoute par composition serde
(`#[serde(flatten)]`) sur son propre type, sans que le moteur en sache rien.

Modes de réponse, tous les quatre présents dès le départ :

| Mode | Ce qu'il fait | Pourquoi il existe |
|---|---|---|
| statique | rend la sortie écrite dans la règle | le cas courant |
| `exec: real` | laisse passer vers le vrai binaire | `jq`, `sops`, `age` sont déterministes et locaux ; ce qu'on veut d'eux est le journal, pas une réponse inventée |
| `exec: <script>` | délègue à un exécutable du projet | l'échappatoire pour la logique à état |
| `render: <script>` | habille la réponse retenue | quand la dépendance parle un format de fil que le YAML ne devinera jamais |

Le quatrième mode a été ajouté au cadrage initial : le brief supposait que
« seule la réponse varie », mais un outil qui répond une enveloppe JSON avec des
compteurs dedans n'entre dans aucun des trois autres. Un outil sans échappatoire
se fait contourner ; c'est la raison pour laquelle les quatre sont là dès le
premier jour.

### `crates/gaveldrop` — `case`

Le format du cas, son chargement, et le schéma JSON qui le décrit.

**Invariant d'architecture :** le schéma JSON est **dérivé du type**, jamais
écrit à la main. Il est commité dans le dépôt et régénéré par un test qui échoue
si le fichier commité a divergé. C'est ce qui rend le format sûr à écrire à la
main et à générer par un agent : un schéma écrit à la main mentirait au premier
changement de forme.

**Invariant d'architecture :** un cas invalide échoue **au chargement**, avec le
nom de la clé fautive. Jamais trois étapes plus loin, avec un message qui parle
d'autre chose. Le coût d'un mauvais message d'erreur ici est payé par tous les
cas jamais écrits.

**Invariant d'architecture :** le noyau ne comprend du bloc `setup` que deux
clés — `run` et `exec`. **Tout le reste y est opaque** et part tel quel dans le
branchement. C'est ce qui laisse armadai écrire `pattern: ring, agents: […]`
sans que gaveldrop sache ce qu'est un motif ou un agent, et sans que le noyau
gagne une once de vocabulaire métier.

**Invariant d'architecture :** chaque assertion porte le **chemin** d'où elle
vient dans le document — `expect.files["…/plugins.yaml"].absent[0]`. Le noyau
n'a pas besoin de numéros de ligne, mais l'annotation de revue de code en aura
besoin plus tard, et remonter d'un chemin à une ligne est facile alors que
reconstruire une provenance qu'on n'a pas gardée ne l'est pas.

### `crates/gaveldrop` — `iso`

L'environnement isolé : un dossier vierge par cas, le dossier personnel et les
dossiers de configuration redirigés dedans, `PATH` préfixé du dossier de liens
vers le faux binaire.

**Invariant d'architecture :** un cas ne voit jamais le vrai dossier personnel.
C'est l'invariant porteur de tout l'édifice — un défaut ici signifie que la
suite de tests corrompt silencieusement la configuration réelle de la personne
qui l'exécute. Toute évolution de ce module se relit avec cette phrase en tête.

**Invariant d'architecture :** une variable d'environnement qui pourrait
contourner la redirection est **effacée**, pas seulement surchargée. Un projet
qui lit `MONOUTIL_CONFIG_DIR` avant de regarder le dossier personnel court-
circuite l'isolation si la variable traîne dans l'environnement de la personne
qui lance les tests. La leçon vient du prototype, qui efface explicitement la
sienne.

**Invariant d'architecture :** l'isolation ne demande **rien** au projet testé.
Pas de variable à lire, pas de mode test, pas de point d'injection. Elle
n'utilise que ce qu'un processus subit de toute façon : son environnement et son
chemin de recherche.

Une photo de l'arborescence est prise après le `setup` et avant l'invocation ;
la différence après invocation constitue l'observation « fichiers ».
**L'observation prend tout** — le dossier est minuscule, le parcourir ne coûte
rien — **et l'assertion nomme des chemins.** Il n'y a donc pas d'arbitrage entre
« différence complète » et « liste surveillée » : ce sont deux étages
différents. En bonus, le rapport d'échec liste les fichiers déposés dont le cas
ne parle pas, non comme une erreur mais comme une aide : c'est souvent là qu'on
découvre ce qu'on aurait dû assérer.

### `crates/gaveldrop` — `adapters`

Un adaptateur a une seule responsabilité : invoquer le sujet et rendre des
observations normalisées.

```rust
pub trait Adapter {
    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations>;
}

pub struct Observations {
    pub exit:   i32,
    pub stdout: String,
    pub stderr: String,
    pub files:  Vec<FileEffect>,
    pub calls:  Vec<Call>,
    pub ext:    BTreeMap<String, Value>,
}
```

**Invariant d'architecture :** un adaptateur invoque et observe. Il **n'évalue
jamais**. Aucun adaptateur ne sait ce qu'un cas attend ; il ne fait que remplir
`Observations`. C'est ce qui garantit qu'une attente écrite une fois se comporte
identiquement quelle que soit la techno.

**Invariant d'architecture :** un adaptateur ne remplit `ext` qu'avec ce que sa
techno **seule** peut produire. Tout ce qui est observable d'un processus
quelconque a déjà sa place dans un champ nommé. `ext` n'est pas un débarras pour
ce qu'on n'a pas eu le courage de placer.

L'adaptateur `process` — lancer une commande, lire ce qu'elle produit — est le
cas de base dont tous les autres sont des spécialisations. Il couvre à lui seul
Rust, JavaScript/TypeScript, Python, Java et Kotlin : dans les cinq, le sujet
testé est un processus, et le faux binaire est indifférent au langage de qui
l'appelle. Le shell est la seule techno qui demande un adaptateur propre, parce
qu'on y teste une fonction et non un exécutable.

Le trait n'a qu'un implémenteur au départ. Il est là quand même : le shell et le
web en ont tous les deux besoin, et c'est lui que le kit de conformité met sous
tension.

### `crates/gaveldrop` — `verdict`

L'évaluation des attentes et des invariants contre `Observations`, et le score
pondéré.

Attentes du noyau : `exit_code` ; `stdout` et `stderr` avec `contains` et
`absent` ; `files`, par chemin, avec `contains` et `absent` ; `calls`, par
comptes. Plus la lecture des lignes JSON émises sur la sortie standard, avec
vérification d'ordre en sous-séquence et de comptes par type.

**Invariant d'architecture :** les invariants nommés ne sont pas du code écrit
par projet. Il y a **quatre formes** intégrées — *apparié*, *exactement un*,
*pas d'orphelin*, *champ non vide* — que la config du projet nomme et paramètre.
Quatre, parce que ce sont exactement celles qui existaient dans le prototype.
Une bibliothèque d'invariants spéculative serait du poids mort ; une cinquième
forme s'ajoute le jour où un cas réel la réclame, pas avant.

**Invariant d'architecture :** un échec nomme le cas, l'attente et la valeur
obtenue. Un message qui oblige à ouvrir le code de gaveldrop est un bug de
gaveldrop, pas un inconfort.

`weight` par cas fait remonter les échecs qui comptent ; `allow_fail` tolère les
cas connus sans les cacher.

### `crates/gaveldrop` — `report`

La sortie terminale, le rapport JSON, le rapport HTML.

**Invariant d'architecture :** le rapport JSON est **une liste de résultats de
cas plus un résumé calculé à partir d'elle**. Jamais un résumé figé en tête de
structure. C'est ce qui rend deux rapports fusionnables par simple
concaténation, et donc ce qui rendra possible de répartir une suite sur
plusieurs machines sans retoucher le format.

**Invariant d'architecture :** les résultats sont émis **au fil de l'eau, un par
cas terminé**, et pas seulement agrégés à la fin. Un rapport qui n'existe qu'une
fois la suite finie interdit toute restitution vivante — un éditeur qui coche ses
cas au fur et à mesure, un terminal qui affiche l'échec dès qu'il tombe. Émettre
une ligne par cas coûte quelques lignes aujourd'hui ; le rétro-adapter
obligerait à retourner la boucle d'exécution.

### `crates/gaveldrop-cli`

La façade en ligne de commande, pour les projets qui ne sont pas en Rust. Elle
lit la config du projet, découvre les cas, exécute, rapporte.

**Invariant d'architecture :** la façade ne contient **aucune logique**. Tout ce
qu'elle fait est disponible depuis la bibliothèque. Un comportement qui n'existe
qu'en passant par le programme est un comportement qu'un projet Rust ne peut pas
tester.

### `crates/gaveldrop-conformance`

Une batterie de cas que tout adaptateur doit passer pour prouver qu'il honore le
contrat : l'isolation n'a pas fui hors du dossier temporaire, `Observations` est
correctement rempli, un appel imprévu déclenche le filet, le journal est
complet.

Le kit a deux usages, et le second est le moins évident : il empêche le noyau de
se déformer quand on ajoute une techno, et il donne à un tiers le moyen de
valider son propre adaptateur sans lire notre code.

**Invariant d'architecture :** le kit de conformité est la garantie propre de
gaveldrop. Le fait qu'un consommateur particulier passe ses tests **n'en est
pas une** : ces cas appartiennent au consommateur, ils peuvent changer sans
préavis, et les copier ici les ferait diverger au premier changement.

### Les branchements — Frontière d'API

Trois points d'extension, un seul protocole. L'exécutable reçoit du JSON sur son
entrée standard et rend son résultat sur sa sortie standard. Le dossier isolé et
le nom du cas lui sont passés par l'environnement.

| Branchement | Reçoit | Rend |
|---|---|---|
| `setup.exec` | le bloc `setup` | rien — son code de sortie fait verdict |
| `fake.render` | la règle retenue et l'appel | les octets que le faux doit émettre |
| `expect.exec` | les observations | `{ "ok": bool, "diffs": [...] }` |

**Invariant d'architecture :** l'unité d'extension est **un exécutable**, pas
une crate Rust. C'est la décision qui met toutes les technos visées sur un pied
d'égalité : un projet Kotlin ou Python peut brancher exactement ce qu'un projet
Rust branche. Si le point d'extension avait été un trait, seul Rust aurait pu
étendre gaveldrop.

**Invariant d'architecture :** le contrat, c'est **le protocole JSON**, pas les
paquets de confort qu'on publiera par écosystème. Un langage sans paquet marche
avec trois lignes de `jq`, et un paquet en retard ne bloque personne. Aucun
paquet n'est publié tant qu'un vrai script de projet n'est pas devenu laid.

**Invariant d'architecture :** trois branchements. Un quatrième est une décision
consciente qu'on justifie ici, pas une dérive qu'on constate.

**Un coût assumé :** `fake.render` est relancé à chaque appel intercepté. Un
script en bash coûte une dizaine de millisecondes, soit quelques secondes sur
une suite chargée. Sensible mais tolérable — et le coût ne tombe que sur les
projets sans alternative : un projet Rust compile son faux binaire avec la
bibliothèque et ne paie rien.

### Le trajet d'un cas

1. Charger et valider le cas contre le schéma.
2. Créer le dossier vierge ; rediriger le dossier personnel et les dossiers de
   configuration dedans.
3. Y poser les liens vers le faux binaire, préfixer `PATH`, écrire le scénario,
   créer le dossier du compteur et du journal.
4. Si le cas a un `setup.exec`, le lancer dans le dossier isolé.
5. Photographier l'arborescence.
6. Laisser l'adaptateur invoquer le sujet.
7. Récolter : code de sortie, sorties, différence d'arborescence, journal.
8. Évaluer les attentes, puis les invariants.
9. Si le cas a un `expect.exec`, le lancer et joindre son verdict.
10. Agréger dans le rapport.

## Préoccupations transverses

### Génération de code

Un seul artefact est généré : le schéma JSON du format de cas, dérivé du type
et commité. Un test le régénère et échoue en cas de divergence. Cette
mécanique est reprise du prototype, où elle a fait la preuve qu'elle tient.

### Tests

Trois niveaux, à trois frontières différentes, et ils ne se remplacent pas :

- **Unitaires**, au plus près : la sélection de règle, le compteur, le
  comparateur d'attentes, la fusion de rapports.
- **Le kit de conformité**, à la frontière de l'adaptateur : la garantie propre
  de gaveldrop, et la seule qui empêche le noyau de se déformer quand on ajoute
  une techno.
- **Les consommateurs réels**, hors de ce dépôt : ce sont eux qui portent la
  preuve de non-régression, chacun chez lui. Elle n'est pas rapatriée ici.

### Gestion d'erreurs

**Invariant d'architecture :** un cas cassé ne fait jamais tomber la suite. Un
dossier temporaire qui refuse de se créer, un programme qui ne démarre pas, un
branchement qui sort en erreur — tout cela devient un cas en échec avec un
diagnostic, pas une panique qui emporte les quatre-vingt-dix-neuf autres cas.

La distinction compte : une erreur de **chargement** est bruyante et arrête tout
(un cas mal écrit est un bug qu'il faut voir tout de suite), une erreur
d'**exécution** est un échec de cas comme un autre.

### Performance

Une seule mesure commande la conception, et c'est le **temps de démarrage du
faux binaire**. Il est relancé à chaque appel intercepté ; une suite sérieuse en
compte plusieurs centaines. Ordres de grandeur, démarrage à vide :

| | démarrage | 500 appels |
|---|---|---|
| Rust ou Go | ~2 ms | ~1 s |
| Node | ~35 ms | ~18 s |
| Python | ~40 ms | ~20 s |
| JVM | ~150 ms | ~1 min 15 |

C'est la différence entre un outil qu'on lance à chaque sauvegarde et un outil
qu'on lance en allant chercher un café. Comme la promesse du projet est que la
couverture devienne bon marché, un outil lent est un outil pour lequel on cesse
d'écrire des cas. **Le faux binaire doit donc être compilé** — ce qui écarte
Node, Python et la JVM pour le cœur, indépendamment de tout goût personnel.

Entre Rust et Go, Rust l'emporte pour trois raisons cumulées : le schéma dérivé
du type ne peut pas mentir, le premier consommateur est en Rust et gagne le
chemin typé gratuitement, et le prototype existant est en Rust — donc le
premier morceau est un déménagement plutôt qu'une réécriture. Go serait un bon
choix pour un projet parti de zéro, et serait même meilleur pour l'étape web.

### Observabilité

Le rapport HTML est repris du prototype dès le premier morceau, plutôt que
repoussé. La raison n'est pas technique : le code existe déjà, et un outil qui
fait régresser son premier consommateur en le migrant démarre mal.

### Intégration aux outils : la CI et l'éditeur sont le même problème

L'intégration continue et l'intégration à un éditeur veulent la même chose sous
deux habillages. Les traiter comme un seul problème évite de construire deux
fois la même plomberie.

**Trois fondations, toutes dans le noyau. Aucun greffon dans le noyau.**

**1. Le schéma dérivé du type** couvre à lui seul l'écriture d'un cas — la
complétion, la validation à la frappe, la documentation au survol — dans
**n'importe quel éditeur qui parle le protocole de langage YAML**, sans une ligne
de code de notre côté. C'est la fondation la plus rentable du projet, et elle est
déjà décidée pour une autre raison.

**Invariant d'architecture :** les commentaires de documentation des types du
format de cas ne sont pas du confort de lecture, ce sont **les infobulles vues
dans l'éditeur**. Ils traversent le schéma jusqu'à l'utilisateur. Un champ mal
documenté est un champ mal utilisé.

**2. La provenance des assertions** — le chemin retenu au chargement, résolu en
numéro de ligne — sert exactement deux consommateurs pour un seul travail : le
commentaire posé sur la bonne ligne d'une revue de code, et le soulignement dans
l'éditeur. Même donnée, même résolution.

**3. Les résultats au fil de l'eau**, plus la découverte des cas sous forme
lisible par machine. C'est précisément ce que réclament les interfaces de test
des éditeurs courants : la liste des cas, puis un flux de résultats. Un greffon
devient alors une couche mince, et une couche mince se maintient.

**Invariant d'architecture :** aucun greffon d'éditeur ne vit dans ce dépôt, et
aucun comportement n'existe uniquement pour un greffon. Un greffon consomme la
découverte de cas et le flux de résultats — rien d'autre. C'est la seule façon de
ne pas avoir à maintenir un greffon par éditeur et par version.

Corollaire pratique : le mode veille — relancer les cas touchés à chaque
sauvegarde — est ce qui rend le choix d'un faux binaire compilé payant au
quotidien. Sans lui, la milliseconde gagnée par appel ne se voit nulle part.

### Nomenclature

Les mots-clés du format et les identifiants sont en **anglais** : le nom est
réservé sur les registres publics, et le français serait un mur pour qui
arriverait de l'extérieur. Ce document est en français parce que son lecteur
principal l'est ; le basculer est un travail mécanique à faire au moment de la
publication.

Un mot sur le vocabulaire : « e2e » décrit mal ce qu'on fait pour une techno
comme le shell, où l'on teste une fonction avec ses dépendances simulées — c'est
plus proche d'un test d'intégration. Le nom des commandes et de la documentation
devra en tenir compte.

## Ce qui n'existe pas encore

Chaque morceau livre quelque chose d'utilisable seul.

1. **Le noyau et le faux binaire.** Tout ce qui est décrit ci-dessus, avec le
   seul adaptateur `process`. Validé par les cas propres de gaveldrop et par le
   kit de conformité.
2. **Le shell.** Sourcer un fichier de configuration, invoquer une fonction,
   observer des fichiers déposés hors du dépôt. C'est le juge de paix de la
   généricité : si le noyau l'absorbe sans se déformer, il est générique.
3. **Le web.** Un sujet qui vit — démarrer, attendre qu'il soit prêt, arrêter
   proprement, réserver un port — des cas en plusieurs étapes, et une seconde
   porte pour les faux : un serveur qui écoute au lieu d'un binaire sur `PATH`.
   Le moteur de règles est le même ; seule la porte change. Placé en troisième
   parce que c'est l'étape qui ajoute le plus de machinerie, et qu'on l'écrira
   mieux avec deux technos déjà passées dessus.
4. **L'intégration continue.** JUnit XML, annotations de revue de code pointant
   la ligne du cas, seuils de gating, sélection et répartition sur plusieurs
   machines. Une action GitHub est essentiellement une trentaine de lignes
   au-dessus du binaire, parce qu'une annotation est une ligne de texte sur la
   sortie standard. C'est ici que la provenance des assertions devient un
   numéro de ligne.
5. **La distribution et les greffons.** Publication de la crate et du binaire,
   schéma publié, documentation d'intégration, mode veille, greffons d'éditeur,
   et les paquets de confort par écosystème — chacun si un besoin réel le
   réclame. Rappel : l'écriture d'un cas est déjà couverte dans tout éditeur dès
   le morceau 1, par le seul fait que le schéma est publié. Un greffon n'ajoute
   que le lancement et le retour visuel.

Deux points restent ouverts et sont volontairement laissés tels quels :

- **Le transport de valeur entre étapes** (« garde l'identifiant renvoyé par la
  première requête »). Indispensable pour tester une API, et c'est exactement
  l'endroit où le YAML voudra des conditions et des calculs. Ce sera le premier
  endroit où il faudra dire non.
- **La base de données d'une API.** On ne l'isole pas en déplaçant un dossier
  personnel. Le branchement de préparation fait le travail — mais c'est une
  délégation, pas une solution.
