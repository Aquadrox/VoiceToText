//! Vocabulaire universel et corrections phonétiques / floues post-transcription.
//!
//! Appliqué juste après la transcription (avant le nettoyage LLM éventuel) afin
//! que **Parakeet**, **Vosk**, **Whisper** et le **mode sans LLM** profitent
//! tous de la même précision sur les noms propres, sigles et jargon :
//!
//! 1. **Pass A — Corrections explicites (`[[replacements]]`, « entendu → écrit »)** :
//!    correspondance insensible à la casse et aux accents sur des mots ou
//!    expressions entières (la plus longue d'abord), en préservant ponctuation,
//!    espaces et élisions françaises (`d'`, `l'`, `qu'`…). Les morceaux corrigés
//!    sont verrouillés pour éviter toute double réécriture.
//! 2. **Pass B — Dictionnaire personnel (`dictionary`)** :
//!    - correspondance exacte (casse/accents ignorés), multi-mots ou recollée
//!      (ex. « voice to text » → « VoiceToText », « aqua drox » → « Aquadrox ») ;
//!    - correspondance phonétique française (`french_phonetic`) et floue bornée
//!      (Levenshtein) avec garde stricte contre les faux positifs :
//!      - mots courts (≤ 3 caractères) : correspondance exacte uniquement ;
//!      - mots outils français de 2 lettres (`si`, `ci`, `et`, `en`…) jamais
//!        réécrits s'ils sont en minuscules ;
//!      - vocabulaire courant français (~240 mots fréquents) jamais altéré par
//!        approximation phonétique/floue ;
//!      - garde d'ambiguïté : si deux entrées du dictionnaire obtiennent le même
//!        meilleur score, le mot d'origine est conservé tel quel.

use std::collections::HashSet;

use crate::config::Replacement;
use crate::text::normalize_phrase;

/// Mots-outils français de 1 ou 2 lettres : jamais réécrits par correspondance
/// exacte en minuscules vers un sigle du dictionnaire (ex. « si » → « SI »,
/// « ci » → « CI », « en » → « EN »).
const FRENCH_SHORT_FUNCTION_WORDS: &[&str] = &[
    "a", "ai", "as", "au", "bu", "ca", "ce", "ci", "de", "du", "en", "es", "et", "eu", "il", "je",
    "la", "le", "lu", "ma", "me", "ne", "ni", "on", "or", "ou", "pu", "sa", "se", "si", "su", "ta",
    "te", "tu", "un", "va", "vu", "y",
];

/// Vocabulaire courant français protégé contre toute réécriture phonétique ou
/// floue (une correspondance exacte explicite dans le dictionnaire reste
/// possible si l'utilisateur a délibérément ajouté le mot).
const COMMON_FRENCH_WORDS: &[&str] = &[
    "abord",
    "achat",
    "acheter",
    "action",
    "affaire",
    "afin",
    "aide",
    "aider",
    "ailleurs",
    "aime",
    "aimer",
    "ainsi",
    "aller",
    "allez",
    "allons",
    "alors",
    "ami",
    "amis",
    "amour",
    "annee",
    "annees",
    "appel",
    "appeler",
    "apres",
    "argent",
    "arrive",
    "arriver",
    "assez",
    "attend",
    "attendre",
    "aucun",
    "aucune",
    "aujourd",
    "aura",
    "aurait",
    "aussi",
    "autant",
    "auto",
    "autour",
    "autre",
    "autres",
    "avait",
    "avant",
    "avec",
    "avenir",
    "avez",
    "avis",
    "avoir",
    "avons",
    "base",
    "beau",
    "beaucoup",
    "besoin",
    "bien",
    "bientot",
    "blanc",
    "bloc",
    "boite",
    "bon",
    "bonjour",
    "bonne",
    "bonsoir",
    "bord",
    "bout",
    "bureau",
    "but",
    "cadre",
    "calme",
    "carte",
    "cause",
    "cela",
    "celle",
    "celles",
    "celui",
    "cent",
    "centre",
    "cependant",
    "certain",
    "certaine",
    "certains",
    "cette",
    "ceux",
    "chacun",
    "chaine",
    "champ",
    "chance",
    "change",
    "changer",
    "chaque",
    "charge",
    "chaud",
    "chaude",
    "chef",
    "chemin",
    "cher",
    "cherche",
    "chercher",
    "chez",
    "chiffre",
    "choix",
    "chose",
    "choses",
    "clair",
    "claire",
    "classe",
    "client",
    "clients",
    "code",
    "coeur",
    "coin",
    "comme",
    "comment",
    "commence",
    "compte",
    "compter",
    "condition",
    "connais",
    "connait",
    "conseil",
    "contact",
    "contenu",
    "continue",
    "continuer",
    "contre",
    "copie",
    "corps",
    "cote",
    "coup",
    "cour",
    "courant",
    "cours",
    "court",
    "courte",
    "cout",
    "coute",
    "creer",
    "croire",
    "crois",
    "dans",
    "date",
    "debut",
    "decide",
    "decider",
    "dedans",
    "dehors",
    "deja",
    "delai",
    "demain",
    "demande",
    "demander",
    "depuis",
    "dernier",
    "derniere",
    "derriere",
    "dessous",
    "dessus",
    "detail",
    "details",
    "deux",
    "devant",
    "devenir",
    "devient",
    "devoir",
    "devrait",
    "difficile",
    "dimanche",
    "dire",
    "direct",
    "directe",
    "direction",
    "dis",
    "discute",
    "discussion",
    "dit",
    "dites",
    "doit",
    "doivent",
    "domaine",
    "donc",
    "donne",
    "donnee",
    "donnees",
    "donner",
    "dont",
    "dossier",
    "double",
    "doute",
    "droit",
    "droite",
    "dure",
    "duree",
    "durer",
    "ecole",
    "ecoute",
    "ecouter",
    "ecran",
    "ecrire",
    "ecrit",
    "effet",
    "effort",
    "egalement",
    "elle",
    "elles",
    "encore",
    "endroit",
    "enfant",
    "enfants",
    "enfin",
    "ensemble",
    "ensuite",
    "entend",
    "entendre",
    "entendu",
    "entier",
    "entre",
    "entree",
    "entrer",
    "envers",
    "environ",
    "envoi",
    "envoyer",
    "equipe",
    "erreur",
    "espace",
    "espere",
    "esprit",
    "essai",
    "essayer",
    "etait",
    "etape",
    "etat",
    "etats",
    "etes",
    "etude",
    "etudier",
    "euro",
    "euros",
    "eviter",
    "exact",
    "exacte",
    "exemple",
    "existe",
    "exister",
    "explique",
    "face",
    "facile",
    "facon",
    "faire",
    "fais",
    "fait",
    "faite",
    "faites",
    "faits",
    "falloir",
    "famille",
    "faut",
    "faux",
    "femme",
    "fenetre",
    "fera",
    "ferme",
    "fermer",
    "fete",
    "feuille",
    "fiche",
    "fichier",
    "fichiers",
    "fille",
    "film",
    "fils",
    "final",
    "finale",
    "finir",
    "finit",
    "fixe",
    "fixer",
    "fois",
    "fonction",
    "fond",
    "font",
    "force",
    "forme",
    "former",
    "fort",
    "forte",
    "frais",
    "francais",
    "francaise",
    "france",
    "frere",
    "froid",
    "front",
    "futur",
    "gagne",
    "gagner",
    "garde",
    "garder",
    "gauche",
    "general",
    "generale",
    "gens",
    "genre",
    "gere",
    "gerer",
    "geste",
    "gestion",
    "gout",
    "grace",
    "grand",
    "grande",
    "grands",
    "grave",
    "gros",
    "grosse",
    "groupe",
    "guerre",
    "guide",
    "habite",
    "habitude",
    "haut",
    "haute",
    "heure",
    "heures",
    "heureux",
    "hier",
    "histoire",
    "hiver",
    "homme",
    "hommes",
    "hors",
    "hotel",
    "idee",
    "idees",
    "image",
    "images",
    "imagine",
    "important",
    "importe",
    "impossible",
    "instant",
    "interet",
    "interne",
    "invite",
    "jamais",
    "janvier",
    "jardin",
    "jaune",
    "jeudi",
    "jeune",
    "jeunes",
    "joie",
    "joli",
    "joue",
    "jouer",
    "jour",
    "journal",
    "journee",
    "jours",
    "juillet",
    "juin",
    "jusqu",
    "jusque",
    "juste",
    "laisser",
    "laisse",
    "lance",
    "lancer",
    "langue",
    "large",
    "lecon",
    "lecture",
    "leger",
    "lent",
    "lente",
    "lequel",
    "lettre",
    "leur",
    "leurs",
    "lever",
    "libre",
    "lien",
    "liens",
    "lieu",
    "lieux",
    "ligne",
    "lignes",
    "limite",
    "lire",
    "liste",
    "livre",
    "livres",
    "local",
    "locale",
    "loin",
    "loisir",
    "long",
    "longue",
    "longtemps",
    "lors",
    "lorsque",
    "lourd",
    "lumiere",
    "lundi",
    "lune",
    "machine",
    "madame",
    "magasin",
    "main",
    "mains",
    "maintenant",
    "maire",
    "mais",
    "maison",
    "maitre",
    "malade",
    "malgre",
    "maman",
    "mange",
    "manger",
    "maniere",
    "manque",
    "manquer",
    "marche",
    "marcher",
    "mardi",
    "mari",
    "marque",
    "mars",
    "matin",
    "matiere",
    "mauvais",
    "mauvaise",
    "medecin",
    "meilleur",
    "meilleure",
    "membre",
    "membres",
    "meme",
    "memes",
    "memoire",
    "mener",
    "merci",
    "mercredi",
    "mere",
    "message",
    "messages",
    "mesure",
    "meteo",
    "metier",
    "metre",
    "mets",
    "mettre",
    "midi",
    "mien",
    "mieux",
    "milieu",
    "mille",
    "million",
    "minute",
    "minutes",
    "mise",
    "mission",
    "mode",
    "modele",
    "modeles",
    "moderne",
    "modifie",
    "moins",
    "mois",
    "moitie",
    "moment",
    "moments",
    "monde",
    "monsieur",
    "montant",
    "monte",
    "monter",
    "montre",
    "montrer",
    "morceau",
    "mort",
    "moteur",
    "motif",
    "mots",
    "mouvement",
    "moyen",
    "moyenne",
    "moyens",
    "musee",
    "musique",
    "nature",
    "naturel",
    "neige",
    "neuf",
    "niveau",
    "noir",
    "noire",
    "nombre",
    "nombreux",
    "nomme",
    "noms",
    "nord",
    "normal",
    "normale",
    "note",
    "noter",
    "notes",
    "notre",
    "nous",
    "nouveau",
    "nouveaux",
    "nouvel",
    "nouvelle",
    "nouvelles",
    "novembre",
    "nuit",
    "numero",
    "objet",
    "obtenir",
    "occasion",
    "occupe",
    "occuper",
    "octobre",
    "oeuvre",
    "offre",
    "offrir",
    "oiseau",
    "ombre",
    "oncle",
    "onze",
    "option",
    "options",
    "orange",
    "ordre",
    "oreille",
    "origine",
    "oublie",
    "oublier",
    "ouest",
    "outil",
    "outils",
    "ouvert",
    "ouverte",
    "ouvrir",
    "page",
    "pages",
    "paix",
    "panier",
    "panne",
    "papier",
    "paquet",
    "paraitre",
    "parc",
    "parce",
    "pardon",
    "pareil",
    "parent",
    "parents",
    "parfois",
    "paris",
    "parle",
    "parler",
    "parmi",
    "parole",
    "part",
    "partage",
    "partager",
    "parti",
    "partie",
    "parties",
    "partir",
    "partout",
    "passage",
    "passe",
    "passer",
    "passion",
    "patiente",
    "patron",
    "pause",
    "pauvre",
    "payer",
    "pays",
    "peine",
    "pendant",
    "pense",
    "pensee",
    "penser",
    "perdre",
    "perdu",
    "pere",
    "periode",
    "permet",
    "permettre",
    "permis",
    "personne",
    "personnes",
    "perte",
    "petit",
    "petite",
    "petits",
    "peut",
    "peuvent",
    "phase",
    "photo",
    "photos",
    "phrase",
    "phrases",
    "piece",
    "pieces",
    "pied",
    "pieds",
    "pierre",
    "place",
    "placer",
    "plage",
    "plaire",
    "plaisir",
    "plan",
    "plans",
    "plante",
    "plat",
    "plein",
    "pleine",
    "pluie",
    "plus",
    "plusieurs",
    "plutot",
    "poche",
    "poids",
    "point",
    "points",
    "police",
    "politique",
    "pont",
    "porte",
    "porter",
    "portes",
    "pose",
    "poser",
    "position",
    "possible",
    "poste",
    "pour",
    "pourquoi",
    "pourra",
    "pourrait",
    "pourtant",
    "pousse",
    "pousser",
    "pouvoir",
    "pratique",
    "precis",
    "precise",
    "prefere",
    "premier",
    "premiere",
    "premiers",
    "prend",
    "prendre",
    "prenez",
    "prennent",
    "prepare",
    "preparer",
    "pres",
    "presence",
    "present",
    "presente",
    "presque",
    "presse",
    "pret",
    "prete",
    "preuve",
    "prevoir",
    "prevu",
    "principe",
    "printemps",
    "pris",
    "prise",
    "prive",
    "privee",
    "prix",
    "probleme",
    "problemes",
    "prochain",
    "prochaine",
    "proche",
    "produit",
    "produits",
    "profil",
    "profite",
    "profond",
    "programme",
    "progres",
    "projet",
    "projets",
    "promet",
    "propos",
    "propose",
    "proposer",
    "propre",
    "propres",
    "protege",
    "prouver",
    "public",
    "publique",
    "puis",
    "puisque",
    "puissant",
    "quand",
    "quant",
    "quart",
    "quartier",
    "quatre",
    "quel",
    "quelle",
    "quelles",
    "quelque",
    "quelques",
    "quels",
    "question",
    "questions",
    "quitte",
    "quitter",
    "quoi",
    "raconte",
    "radio",
    "raison",
    "raisons",
    "rapide",
    "rapides",
    "rappel",
    "rappelle",
    "rappeler",
    "rapport",
    "rare",
    "rayon",
    "realise",
    "realiser",
    "realite",
    "recent",
    "recette",
    "recevoir",
    "recherche",
    "recoit",
    "recu",
    "regarde",
    "regarder",
    "region",
    "regle",
    "regles",
    "reglage",
    "reglages",
    "rejoint",
    "relation",
    "remettre",
    "remis",
    "remplace",
    "remplir",
    "rencontre",
    "rend",
    "rendre",
    "rendu",
    "rentre",
    "rentrer",
    "repare",
    "reparer",
    "repas",
    "repeter",
    "repond",
    "repondre",
    "reponse",
    "reponses",
    "repos",
    "reprend",
    "reprendre",
    "reseau",
    "reserve",
    "resoudre",
    "respect",
    "reste",
    "rester",
    "resultat",
    "resultats",
    "retard",
    "retenir",
    "retire",
    "retirer",
    "retour",
    "retourne",
    "retrouve",
    "reunion",
    "reunions",
    "reussi",
    "reussir",
    "reve",
    "reveil",
    "revenir",
    "revient",
    "revoir",
    "riche",
    "rien",
    "rire",
    "risque",
    "risques",
    "riviere",
    "robe",
    "role",
    "roman",
    "rond",
    "rose",
    "roue",
    "rouge",
    "route",
    "routine",
    "rude",
    "ruse",
    "sable",
    "sache",
    "sais",
    "saison",
    "sait",
    "salaire",
    "salle",
    "salon",
    "salut",
    "samedi",
    "sang",
    "sans",
    "sante",
    "sauve",
    "sauver",
    "savait",
    "savoir",
    "savons",
    "scene",
    "science",
    "score",
    "seance",
    "second",
    "seconde",
    "secondes",
    "secret",
    "secteur",
    "section",
    "securite",
    "seize",
    "selon",
    "semaine",
    "semaines",
    "semble",
    "sembler",
    "sens",
    "sent",
    "sentir",
    "sept",
    "septembre",
    "sera",
    "serai",
    "serait",
    "serieux",
    "serie",
    "seront",
    "sert",
    "service",
    "services",
    "servir",
    "seul",
    "seule",
    "seulement",
    "seules",
    "seuls",
    "siecle",
    "siege",
    "sien",
    "signe",
    "signer",
    "silence",
    "simple",
    "simples",
    "sinon",
    "site",
    "sites",
    "situation",
    "social",
    "sociale",
    "societe",
    "soeur",
    "soin",
    "soins",
    "soir",
    "soiree",
    "soit",
    "soixante",
    "soleil",
    "solide",
    "solution",
    "somme",
    "sommes",
    "sondage",
    "sont",
    "sort",
    "sorte",
    "sorti",
    "sortie",
    "sortir",
    "souci",
    "soucis",
    "soudain",
    "souffle",
    "souhaite",
    "source",
    "sources",
    "sourire",
    "sous",
    "soutien",
    "souvenir",
    "souvent",
    "special",
    "sport",
    "stade",
    "stage",
    "station",
    "statut",
    "stock",
    "style",
    "styles",
    "subir",
    "succes",
    "sucre",
    "suffit",
    "suis",
    "suite",
    "suivant",
    "suivante",
    "suivi",
    "suivre",
    "sujet",
    "sujets",
    "super",
    "support",
    "suppose",
    "supprime",
    "sure",
    "surface",
    "surprise",
    "surtout",
    "systeme",
    "table",
    "tableau",
    "tache",
    "taches",
    "taille",
    "talent",
    "tandis",
    "tant",
    "tante",
    "tape",
    "taper",
    "tapis",
    "tard",
    "tarif",
    "tarifs",
    "tasse",
    "taux",
    "taxe",
    "technique",
    "telle",
    "telles",
    "tels",
    "temoin",
    "temps",
    "tend",
    "tendre",
    "tenir",
    "tension",
    "tente",
    "tenter",
    "tenu",
    "tenue",
    "terme",
    "termes",
    "termine",
    "terminer",
    "terrain",
    "terre",
    "test",
    "teste",
    "tester",
    "tests",
    "tete",
    "texte",
    "textes",
    "theme",
    "ticket",
    "tien",
    "tiens",
    "tient",
    "tiers",
    "timbre",
    "tire",
    "tirer",
    "titre",
    "titres",
    "toile",
    "toit",
    "tombe",
    "tomber",
    "tonne",
    "tort",
    "total",
    "totale",
    "touche",
    "toucher",
    "toujours",
    "tour",
    "tourne",
    "tourner",
    "tours",
    "tous",
    "tout",
    "toute",
    "toutes",
    "trace",
    "traduire",
    "train",
    "traite",
    "traiter",
    "trajet",
    "tranche",
    "travail",
    "travailler",
    "travers",
    "treize",
    "trente",
    "tres",
    "triste",
    "trois",
    "trop",
    "trou",
    "trouble",
    "trouve",
    "trouver",
    "truc",
    "tube",
    "tuer",
    "type",
    "types",
    "union",
    "unique",
    "unite",
    "univers",
    "urgent",
    "usage",
    "usine",
    "utile",
    "utilise",
    "utiliser",
    "vacances",
    "vague",
    "vaincre",
    "vais",
    "valeur",
    "valeurs",
    "valide",
    "valider",
    "valise",
    "valoir",
    "vaste",
    "vaut",
    "vecu",
    "veille",
    "vend",
    "vendre",
    "vendredi",
    "vendu",
    "venir",
    "vent",
    "vente",
    "ventes",
    "ventre",
    "venu",
    "venue",
    "verbe",
    "verifie",
    "verifier",
    "verite",
    "verra",
    "verre",
    "vers",
    "version",
    "versions",
    "vert",
    "verte",
    "veste",
    "veulent",
    "veut",
    "veux",
    "vide",
    "video",
    "videos",
    "vider",
    "vieil",
    "vieille",
    "viendra",
    "viens",
    "vient",
    "vieux",
    "vigne",
    "village",
    "ville",
    "villes",
    "vingt",
    "virgule",
    "visage",
    "vise",
    "viser",
    "vision",
    "visite",
    "visiter",
    "vite",
    "vitesse",
    "vitre",
    "vivant",
    "vive",
    "vivre",
    "voici",
    "voie",
    "voila",
    "voile",
    "voir",
    "vois",
    "voisin",
    "voit",
    "voiture",
    "voix",
    "voler",
    "volet",
    "volonte",
    "volume",
    "vont",
    "vote",
    "voter",
    "votre",
    "voudrais",
    "voulait",
    "vouloir",
    "voulu",
    "vous",
    "voyage",
    "voyager",
    "voyons",
    "vrai",
    "vraie",
    "vraiment",
    "zone",
    "zones",
];

fn is_short_function_word(norm: &str) -> bool {
    norm.len() <= 2 && FRENCH_SHORT_FUNCTION_WORDS.binary_search(&norm).is_ok()
}

fn is_common_french_word(norm: &str) -> bool {
    is_short_function_word(norm) || COMMON_FRENCH_WORDS.contains(&norm)
}

/// Normalise un mot unique en minuscules ASCII sans accents (alphanumérique uniquement).
fn normalize_word(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'â' | 'ä' | 'á' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'î' | 'ï' | 'í' => out.push('i'),
            'ô' | 'ö' | 'ó' => out.push('o'),
            'ù' | 'û' | 'ü' | 'ú' => out.push('u'),
            'ÿ' => out.push('y'),
            'ç' => out.push('c'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

fn is_vowel_byte(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

/// Clé phonétique française légère, déterministe et sans dépendance.
///
/// L'entrée est supposée déjà normalisée (`normalize_word` : minuscules sans
/// accents). Les voyelles nasales et digrammes sont encodés par des lettres
/// majuscules (`A`, `E`, `I`, `N`, `O`, `U`, `X`) pour rester distincts des
/// consonnes simples.
pub fn french_phonetic(word: &str) -> String {
    let norm = normalize_word(word);
    if norm.is_empty() {
        return String::new();
    }
    let bytes = norm.as_bytes();
    let mut raw = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        let rest = &bytes[i..];
        if !bytes[i].is_ascii() {
            // Préserve tel quel un caractère non-ASCII alphanumérique éventuel.
            let ch = norm[i..].chars().next().unwrap();
            raw.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let next_blocks_nasal = |after: usize| {
            bytes
                .get(after)
                .is_some_and(|&b| is_vowel_byte(b) || b == b'n' || b == b'm')
        };

        if rest.starts_with(b"eau") {
            raw.push('o');
            i += 3;
        } else if rest.starts_with(b"oeu") {
            raw.push('E');
            i += 3;
        } else if (rest.starts_with(b"ain")
            || rest.starts_with(b"aim")
            || rest.starts_with(b"ein")
            || rest.starts_with(b"eim"))
            && !next_blocks_nasal(i + 3)
        {
            raw.push('I');
            i += 3;
        } else if rest.starts_with(b"ch") {
            raw.push('X');
            i += 2;
        } else if rest.starts_with(b"ph") {
            raw.push('f');
            i += 2;
        } else if rest.starts_with(b"qu") {
            raw.push('k');
            i += 2;
        } else if rest.starts_with(b"gu")
            && bytes
                .get(i + 2)
                .is_some_and(|&b| matches!(b, b'e' | b'i' | b'y'))
        {
            raw.push('g');
            i += 2;
        } else if rest.starts_with(b"gn") {
            raw.push('N');
            i += 2;
        } else if rest.starts_with(b"au") {
            raw.push('o');
            i += 2;
        } else if rest.starts_with(b"ou") {
            raw.push('U');
            i += 2;
        } else if rest.starts_with(b"ee")
            && bytes
                .get(i + 2)
                .is_some_and(|&b| b.is_ascii_alphabetic() && !is_vowel_byte(b))
        {
            // « ee » devant consonne (anglicismes : Parakeet, DeepSeek, Meet…) → /i/.
            raw.push('i');
            i += 2;
        } else if rest.starts_with(b"eu") {
            raw.push('E');
            i += 2;
        } else if rest.starts_with(b"ai") || rest.starts_with(b"ei") {
            raw.push('e');
            i += 2;
        } else if rest.starts_with(b"oi") {
            raw.push_str("wa");
            i += 2;
        } else if (rest.starts_with(b"an")
            || rest.starts_with(b"am")
            || rest.starts_with(b"en")
            || rest.starts_with(b"em"))
            && !next_blocks_nasal(i + 2)
        {
            raw.push('A');
            i += 2;
        } else if (rest.starts_with(b"in")
            || rest.starts_with(b"im")
            || rest.starts_with(b"yn")
            || rest.starts_with(b"ym")
            || rest.starts_with(b"un")
            || rest.starts_with(b"um"))
            && !next_blocks_nasal(i + 2)
        {
            raw.push('I');
            i += 2;
        } else if (rest.starts_with(b"on") || rest.starts_with(b"om")) && !next_blocks_nasal(i + 2)
        {
            raw.push('O');
            i += 2;
        } else if rest.starts_with(b"ss")
            || (rest.starts_with(b"sc")
                && bytes
                    .get(i + 2)
                    .is_some_and(|&b| matches!(b, b'e' | b'i' | b'y')))
        {
            raw.push('s');
            i += 2;
        } else {
            let b = bytes[i];
            match b {
                b'c' => {
                    if bytes
                        .get(i + 1)
                        .is_some_and(|&n| matches!(n, b'e' | b'i' | b'y'))
                    {
                        raw.push('s');
                    } else {
                        raw.push('k');
                    }
                }
                b'g' => {
                    if bytes
                        .get(i + 1)
                        .is_some_and(|&n| matches!(n, b'e' | b'i' | b'y'))
                    {
                        raw.push('j');
                    } else {
                        raw.push('g');
                    }
                }
                b's' => {
                    let prev_v = i > 0 && is_vowel_byte(bytes[i - 1]);
                    let next_v = bytes.get(i + 1).is_some_and(|&n| is_vowel_byte(n));
                    if prev_v && next_v {
                        raw.push('z');
                    } else {
                        raw.push('s');
                    }
                }
                b'x' => raw.push_str("ks"),
                b'w' => raw.push('v'),
                b'y' => raw.push('i'),
                b'q' => raw.push('k'),
                b'h' => {} // h muet en français (ch/ph déjà traités)
                _ => raw.push(b as char),
            }
            i += 1;
        }
    }

    // Fusion des lettres consécutives identiques.
    let mut collapsed = String::with_capacity(raw.len());
    let mut prev: Option<char> = None;
    for ch in raw.chars() {
        if prev != Some(ch) {
            collapsed.push(ch);
            prev = Some(ch);
        }
    }

    // Lettres finales généralement muettes en français (`e`, `s`, `es`), hors fin en « x » (Aquadrox).
    let len = collapsed.chars().count();
    if len > 3 && norm.ends_with("es") && collapsed.ends_with('s') {
        collapsed.pop();
        if collapsed.chars().count() > 3 && collapsed.ends_with('e') {
            collapsed.pop();
        }
    } else if len > 3
        && ((norm.ends_with('e') && collapsed.ends_with('e'))
            || (norm.ends_with('s') && !norm.ends_with("ss") && collapsed.ends_with('s')))
    {
        collapsed.pop();
    }

    collapsed
}

/// Distance d'édition de Levenshtein (en caractères Unicode).
fn levenshtein(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    let b_chars: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a.chars().count();
    }
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0; b_chars.len() + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

/// Un mot du texte d'origine avec sa tranche exacte en octets et sa forme normalisée.
#[derive(Debug)]
struct Token<'a> {
    start: usize,
    end: usize,
    raw: &'a str,
    norm: String,
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            if start.is_none() {
                start = Some(idx);
            }
        } else if let Some(s) = start.take() {
            let raw = &text[s..idx];
            let norm = normalize_word(raw);
            if !norm.is_empty() {
                tokens.push(Token {
                    start: s,
                    end: idx,
                    raw,
                    norm,
                });
            }
        }
    }
    if let Some(s) = start {
        let raw = &text[s..];
        let norm = normalize_word(raw);
        if !norm.is_empty() {
            tokens.push(Token {
                start: s,
                end: text.len(),
                raw,
                norm,
            });
        }
    }
    tokens
}

/// Séparateur autorisé au sein d'une correspondance multi-mots explicite ou
/// dictionnaire (espaces, tiret, apostrophe, point interne ; jamais de saut de
/// ligne ni de ponctuation de rupture de proposition).
fn is_phrase_gap(gap: &str) -> bool {
    !gap.is_empty()
        && gap
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | '\u{a0}' | '-' | '\'' | '’' | '.'))
}

/// Séparateur autorisé pour recoller un mot découpé par l'ASR (« aqua drox »,
/// « voice to text », « kuber-nettes ») : espace ou tiret uniquement, jamais
/// d'apostrophe (pour ne jamais recoller « d'aquatroc » en « daquatroc »).
fn is_split_word_gap(gap: &str) -> bool {
    !gap.is_empty()
        && gap
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | '\u{a0}' | '-'))
        && gap.chars().filter(|&c| c == '-').count() <= 1
}

/// Adapte la casse de la cible : si la cible contient une majuscule délibérée
/// (`Aquadrox`, `PostgreSQL`, `iOS`, `API`), elle est gardée telle quelle ; si
/// elle est entièrement en minuscules (`tokio`, `serde`) et que le texte
/// d'origine commençait par une majuscule, on capitalise la première lettre.
fn apply_casing(original: &str, target: &str) -> String {
    if target.chars().any(char::is_uppercase) {
        return target.to_owned();
    }
    let starts_upper = original.chars().next().is_some_and(char::is_uppercase);
    if !starts_upper {
        return target.to_owned();
    }
    let mut chars = target.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Clone)]
struct CompiledReplacement {
    heard_tokens: Vec<String>,
    written: String,
}

#[derive(Debug, Clone)]
struct CompiledDictEntry {
    written: String,
    tokens: Vec<String>,
    /// Tous les caractères alphanumériques normalisés collés (ex. « voicetotext »).
    collapsed: String,
    /// Clé phonétique française (uniquement pour les entrées à 1 mot ou collables).
    phonetic: String,
    /// L'entrée a une majuscule interne, un tiret ou un point (ex. `VoiceToText`, `Wi-Fi`, `Node.js`).
    is_styled_compound: bool,
}

/// Vocabulaire compilé (dictionnaire + corrections « entendu → écrit »),
/// conservé dans [`crate::Pipeline`] pour des applications rapides sans
/// réallocation de tables à chaque dictée.
#[derive(Debug, Clone, Default)]
pub struct CompiledVocabulary {
    replacements: Vec<CompiledReplacement>,
    dict_multi: Vec<CompiledDictEntry>,
    dict_single: Vec<CompiledDictEntry>,
    /// Ensemble des formes exactes du dictionnaire (en minuscules normalisées),
    /// pour éviter qu'un mot déjà exact dans le dictionnaire soit réécrit vers
    /// un autre mot proche.
    exact_dict_words: HashSet<String>,
}

impl CompiledVocabulary {
    pub fn new(dictionary: &[String], replacements: &[Replacement]) -> Self {
        let mut compiled_replacements = Vec::new();
        for rep in replacements {
            let written = rep.written.trim();
            if written.is_empty() {
                continue;
            }
            let phrase = normalize_phrase(&rep.heard);
            let heard_tokens: Vec<String> = phrase.split_whitespace().map(str::to_owned).collect();
            if heard_tokens.is_empty() {
                continue;
            }
            compiled_replacements.push(CompiledReplacement {
                heard_tokens,
                written: written.to_owned(),
            });
        }
        // Plus longues expressions « entendu » en premier pour éviter qu'un mot
        // seul ne masque une règle multi-mots.
        compiled_replacements.sort_by(|a, b| {
            b.heard_tokens
                .len()
                .cmp(&a.heard_tokens.len())
                .then_with(|| {
                    let len_b: usize = b.heard_tokens.iter().map(String::len).sum();
                    let len_a: usize = a.heard_tokens.iter().map(String::len).sum();
                    len_b.cmp(&len_a)
                })
        });

        let mut dict_multi = Vec::new();
        let mut dict_single = Vec::new();
        let mut exact_dict_words = HashSet::new();

        for raw_entry in dictionary {
            let written = raw_entry.trim();
            if written.is_empty() {
                continue;
            }
            let phrase = normalize_phrase(written);
            let tokens: Vec<String> = phrase.split_whitespace().map(str::to_owned).collect();
            if tokens.is_empty() {
                continue;
            }
            let collapsed: String = tokens.iter().map(String::as_str).collect();
            let phonetic = french_phonetic(&collapsed);
            let has_internal_upper = written.chars().skip(1).any(char::is_uppercase);
            let has_compound_punct = written.contains('-') || written.contains('.');
            let entry = CompiledDictEntry {
                written: written.to_owned(),
                tokens: tokens.clone(),
                collapsed,
                phonetic,
                is_styled_compound: has_internal_upper || has_compound_punct,
            };
            if tokens.len() > 1 {
                dict_multi.push(entry);
            } else {
                exact_dict_words.insert(tokens[0].clone());
                dict_single.push(entry);
            }
        }
        dict_multi.sort_by_key(|a| std::cmp::Reverse(a.tokens.len()));

        Self {
            replacements: compiled_replacements,
            dict_multi,
            dict_single,
            exact_dict_words,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty() && self.dict_multi.is_empty() && self.dict_single.is_empty()
    }

    /// Applique les corrections explicites puis le dictionnaire personnel au
    /// texte transcrit, en préservant ponctuation, retours à la ligne,
    /// espaces et élisions françaises.
    pub fn apply(&self, text: &str) -> String {
        if self.is_empty() || text.is_empty() {
            return text.to_owned();
        }
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return text.to_owned();
        }

        // Remplacements planifiés : (index début token, index fin exclusif, texte de remplacement).
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        let mut locked = vec![false; tokens.len()];

        // Pass A : corrections explicites « entendu → écrit ».
        if !self.replacements.is_empty() {
            let mut i = 0;
            while i < tokens.len() {
                if locked[i] {
                    i += 1;
                    continue;
                }
                let mut matched = None;
                for rule in &self.replacements {
                    let len = rule.heard_tokens.len();
                    if i + len > tokens.len() {
                        continue;
                    }
                    if locked[i..i + len].iter().any(|&l| l) {
                        continue;
                    }
                    let all_match = (0..len).all(|k| {
                        tokens[i + k].norm == rule.heard_tokens[k]
                            && (k == 0
                                || is_phrase_gap(&text[tokens[i + k - 1].end..tokens[i + k].start]))
                    });
                    if all_match {
                        matched = Some((len, rule.written.as_str()));
                        break;
                    }
                }
                if let Some((len, written)) = matched {
                    let replacement = apply_casing(tokens[i].raw, written);
                    edits.push((i, i + len, replacement));
                    for l in &mut locked[i..i + len] {
                        *l = true;
                    }
                    i += len;
                } else {
                    i += 1;
                }
            }
        }

        // Pass B : dictionnaire personnel (exact multi-mots, recollé, exact simple,
        // puis phonétique / flou avec gardes anti-faux-positifs).
        if !self.dict_multi.is_empty() || !self.dict_single.is_empty() {
            let mut i = 0;
            while i < tokens.len() {
                if locked[i] {
                    i += 1;
                    continue;
                }
                if let Some((len, written)) = self.match_dict_at(text, &tokens, &locked, i) {
                    let replacement = apply_casing(tokens[i].raw, written);
                    let original_span = &text[tokens[i].start..tokens[i + len - 1].end];
                    if replacement != original_span {
                        edits.push((i, i + len, replacement));
                    }
                    for l in &mut locked[i..i + len] {
                        *l = true;
                    }
                    i += len;
                } else {
                    i += 1;
                }
            }
        }

        if edits.is_empty() {
            return text.to_owned();
        }
        edits.sort_by_key(|(start, _, _)| *start);

        let mut out = String::with_capacity(text.len() + 16);
        let mut cursor = 0;
        for (tok_start, tok_end, replacement) in edits {
            let byte_start = tokens[tok_start].start;
            let byte_end = tokens[tok_end - 1].end;
            out.push_str(&text[cursor..byte_start]);
            out.push_str(&replacement);
            cursor = byte_end;
        }
        out.push_str(&text[cursor..]);
        out
    }

    fn match_dict_at(
        &self,
        text: &str,
        tokens: &[Token<'_>],
        locked: &[bool],
        i: usize,
    ) -> Option<(usize, &str)> {
        // 1. Entrée multi-mots exacte du dictionnaire (ex. « Visual Studio », « Wi-Fi », « Jean-Luc »).
        for entry in &self.dict_multi {
            let len = entry.tokens.len();
            if i + len <= tokens.len()
                && !locked[i..i + len].iter().any(|&l| l)
                && (0..len).all(|k| {
                    tokens[i + k].norm == entry.tokens[k]
                        && (k == 0
                            || is_phrase_gap(&text[tokens[i + k - 1].end..tokens[i + k].start]))
                })
            {
                return Some((len, entry.written.as_str()));
            }
        }

        // 2. Mot composé ou découpé par l'ASR (2 ou 3 tokens séparés par espace/tiret)
        // correspondant exactement à la forme collée d'une entrée du dictionnaire
        // (ex. « voice to text » → « VoiceToText », « aqua drox » → « Aquadrox »).
        for span in [3usize, 2usize] {
            if i + span > tokens.len() || locked[i..i + span].iter().any(|&l| l) {
                continue;
            }
            let gaps_ok = (1..span)
                .all(|k| is_split_word_gap(&text[tokens[i + k - 1].end..tokens[i + k].start]));
            if !gaps_ok {
                continue;
            }
            let joined: String = tokens[i..i + span]
                .iter()
                .map(|t| t.norm.as_str())
                .collect();
            if joined.chars().count() < 5 {
                continue;
            }
            let mut found: Option<&str> = None;
            let mut ambiguous = false;
            for entry in self.dict_single.iter().chain(self.dict_multi.iter()) {
                if entry.collapsed == joined
                    && (entry.is_styled_compound
                        || (joined.chars().count() >= 6 && !is_common_french_word(&joined)))
                {
                    if found.is_some() {
                        ambiguous = true;
                        break;
                    }
                    found = Some(entry.written.as_str());
                }
            }
            if !ambiguous && let Some(written) = found {
                return Some((span, written));
            }
        }

        // 3. Correspondance exacte sur un mot unique du dictionnaire.
        let tok = &tokens[i];
        let tok_all_upper = tok
            .raw
            .chars()
            .all(|c| !c.is_alphabetic() || c.is_uppercase());
        for entry in &self.dict_single {
            if entry.tokens[0] == tok.norm {
                // Garde sur les mots-outils français de 1 ou 2 lettres : ni
                // « en » / « En » (début de phrase), ni « si » ne deviennent
                // « EN » / « SI » ; seuls « CI » déjà en majuscules ou les
                // sigles non outils (« l'ia » → « l'IA ») sont réécrits.
                if is_short_function_word(&tok.norm) && !tok_all_upper {
                    return None;
                }
                return Some((1, entry.written.as_str()));
            }
        }

        // 4. Recollage phonétique sur 2 tokens (ex. « kuber nettes » → « Kubernetes »,
        // « para kite » → « Parakeet »), seulement si les deux tokens ne sont pas
        // tous les deux des mots courants français.
        if i + 2 <= tokens.len()
            && !locked[i + 1]
            && is_split_word_gap(&text[tokens[i].end..tokens[i + 1].start])
        {
            let t1 = &tokens[i].norm;
            let t2 = &tokens[i + 1].norm;
            let both_common = is_common_french_word(t1) && is_common_french_word(t2);
            let either_exact_dict =
                self.exact_dict_words.contains(t1) || self.exact_dict_words.contains(t2);
            if !both_common && !either_exact_dict && !t1.is_empty() && !t2.is_empty() {
                let joined = format!("{t1}{t2}");
                let joined_len = joined.chars().count();
                if joined_len >= 6 && !is_common_french_word(&joined) {
                    let joined_phon = french_phonetic(&joined);
                    if let Some(written) = self.best_split_phonetic_match(
                        &joined,
                        &joined_phon,
                        !is_common_french_word(t1) && !is_common_french_word(t2),
                    ) {
                        return Some((2, written));
                    }
                }
            }
        }

        // 5. Correspondance phonétique / floue sur un mot unique.
        // Gardes strictes anti-faux-positifs :
        // - jamais sur les mots courts (≤ 3 caractères) ;
        // - jamais sur le vocabulaire courant français ;
        // - jamais sur un mot purement numérique.
        let tok_len = tok.norm.chars().count();
        if tok_len < 4
            || is_common_french_word(&tok.norm)
            || tok.norm.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let tok_phon = french_phonetic(&tok.norm);
        self.best_single_fuzzy_match(&tok.norm, &tok_phon)
            .map(|written| (1, written))
    }

    fn best_split_phonetic_match(
        &self,
        joined: &str,
        joined_phon: &str,
        neither_common: bool,
    ) -> Option<&str> {
        let joined_len = joined.chars().count();
        let mut best: Option<((u8, usize, usize), &str)> = None;
        let mut tied = false;

        for entry in &self.dict_single {
            let e_norm = &entry.tokens[0];
            let e_len = e_norm.chars().count();
            let min_len = joined_len.min(e_len);
            let max_len = joined_len.max(e_len);
            if min_len < 6 || joined_len.abs_diff(e_len) > 2 {
                continue;
            }
            let p_dist = levenshtein(joined_phon, &entry.phonetic);
            let edit = levenshtein(joined, e_norm);
            let sim = 1.0 - (edit as f32 / max_len as f32);

            let tier = if p_dist == 0 && edit <= 3 && sim >= 0.65 {
                Some(0)
            } else if neither_common && p_dist == 1 && min_len >= 7 && edit <= 2 && sim >= 0.75 {
                Some(1)
            } else {
                None
            };
            let Some(tier) = tier else {
                continue;
            };
            let score = (tier, p_dist, edit);
            match best {
                None => best = Some((score, entry.written.as_str())),
                Some((best_score, _)) if score < best_score => {
                    best = Some((score, entry.written.as_str()));
                    tied = false;
                }
                Some((best_score, _)) if score == best_score => {
                    tied = true;
                }
                _ => {}
            }
        }
        if tied {
            None
        } else {
            best.map(|(_, written)| written)
        }
    }

    fn best_single_fuzzy_match(&self, tok_norm: &str, tok_phon: &str) -> Option<&str> {
        let t_len = tok_norm.chars().count();
        let t_first_phon = tok_phon.chars().next();
        let mut best: Option<((u8, usize, usize), &str)> = None;
        let mut tied = false;

        for entry in &self.dict_single {
            let e_norm = &entry.tokens[0];
            let e_len = e_norm.chars().count();
            let min_len = t_len.min(e_len);
            let max_len = t_len.max(e_len);
            if min_len < 4 || t_len.abs_diff(e_len) > 2 {
                continue;
            }
            let edit = levenshtein(tok_norm, e_norm);
            let sim = 1.0 - (edit as f32 / max_len as f32);
            let p_dist = levenshtein(tok_phon, &entry.phonetic);
            let same_onset =
                t_first_phon.is_some() && t_first_phon == entry.phonetic.chars().next();

            let tier = if p_dist == 0 && min_len >= 4 && edit <= 3 && sim >= 0.50 && same_onset {
                // Tier 0 : même clé phonétique française (ex. « taurie » → « Tauri »,
                // « parakite » / « paraquit » → « Parakeet », « cubernetes » → « Kubernetes »,
                // « ciléro » → « Silero », « vosque » → « Vosk »).
                Some(0)
            } else if edit == 1 && min_len >= 5 && p_dist <= 1 {
                // Tier 1 : faute d'une seule lettre sur mot ≥ 5 caractères
                // (ex. « aquadroc » → « Aquadrox », « silerot » → « Silero »).
                Some(1)
            } else if p_dist == 1 && min_len >= 6 && edit <= 2 && sim >= 0.70 {
                // Tier 2 : distance phonétique 1 sur mot long ≥ 6 caractères.
                Some(2)
            } else if min_len >= 8 && edit <= 2 && sim >= 0.75 && p_dist <= 2 && same_onset {
                // Tier 3 : confusion voisée/sourde sur mot très long ≥ 8 caractères
                // avec même attaque phonétique (ex. « aquatroc » → « Aquadrox »).
                Some(3)
            } else {
                None
            };
            let Some(tier) = tier else {
                continue;
            };
            let score = (tier, p_dist, edit);
            match best {
                None => best = Some((score, entry.written.as_str())),
                Some((best_score, _)) if score < best_score => {
                    best = Some((score, entry.written.as_str()));
                    tied = false;
                }
                Some((best_score, _)) if score == best_score => {
                    tied = true;
                }
                _ => {}
            }
        }
        if tied {
            None
        } else {
            best.map(|(_, written)| written)
        }
    }
}

/// Applique le dictionnaire personnel et les corrections explicites
/// « entendu → écrit » au texte transcrit.
pub fn apply(text: &str, dictionary: &[String], replacements: &[Replacement]) -> String {
    CompiledVocabulary::new(dictionary, replacements).apply(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| (*s).to_owned()).collect()
    }

    fn rep(heard: &str, written: &str) -> Replacement {
        Replacement {
            heard: heard.into(),
            written: written.into(),
        }
    }

    #[test]
    fn phonetic_key_captures_french_equivalences() {
        assert_eq!(french_phonetic("Tauri"), french_phonetic("taurie"));
        assert_eq!(french_phonetic("Parakeet"), french_phonetic("parakite"));
        assert_eq!(french_phonetic("Parakeet"), french_phonetic("paraquit"));
        assert_eq!(french_phonetic("DeepSeek"), french_phonetic("dipsik"));
        assert_eq!(french_phonetic("Kubernetes"), french_phonetic("cubernetes"));
        assert_eq!(
            french_phonetic("Kubernetes"),
            french_phonetic("kubernettes")
        );
        assert_eq!(french_phonetic("Silero"), french_phonetic("ciléro"));
        assert_eq!(french_phonetic("Vosk"), french_phonetic("vosque"));
        assert_eq!(french_phonetic("Claude"), french_phonetic("clode"));
        assert_eq!(french_phonetic("Whisper"), french_phonetic("wisper"));
    }

    #[test]
    fn explicit_replacements_longest_first_and_locked() {
        let d = dict(&["Aquadrox", "PostgreSQL"]);
        let r = vec![
            rep("poste graisse", "PostgreSQL"),
            rep("poste", "Poste"),
            rep("c'est plus plus", "C++"),
            rep("aqua troc", "Aquadrox"),
        ];
        // « poste graisse » doit gagner sur « poste » (plus longue expression d'abord).
        assert_eq!(
            apply("J'utilise poste graisse et la poste.", &d, &r),
            "J'utilise PostgreSQL et la Poste."
        );
        assert_eq!(
            apply("Le projet d'aqua troc en c'est plus plus !", &d, &r),
            "Le projet d'Aquadrox en C++ !"
        );
    }

    #[test]
    fn realistic_parakeet_and_vosk_errors_are_corrected() {
        let d = dict(&[
            "Aquadrox",
            "Parakeet",
            "Kubernetes",
            "Tauri",
            "VoiceToText",
            "DeepSeek",
            "Silero",
            "Vosk",
            "Whisper",
            "PostgreSQL",
            "API",
            "IA",
        ]);
        let r = vec![rep("poste graisse", "PostgreSQL")];

        assert_eq!(
            apply("Le dépôt d'aquatroc tourne sous taurie.", &d, &r),
            "Le dépôt d'Aquadrox tourne sous Tauri."
        );
        assert_eq!(
            apply("J'ai testé parakite et paraquit avec vosque.", &d, &r),
            "J'ai testé Parakeet et Parakeet avec Vosk."
        );
        assert_eq!(
            apply("Déploiement cubernetes ou kuber nettes sur l'api.", &d, &r),
            "Déploiement Kubernetes ou Kubernetes sur l'API."
        );
        assert_eq!(
            apply("voice to text et deep seek avec aqua drox", &d, &r),
            "VoiceToText et DeepSeek avec Aquadrox"
        );
        assert_eq!(
            apply("ciléro, wisper et poste graisse pour l'ia.", &d, &r),
            "Silero, Whisper et PostgreSQL pour l'IA."
        );
    }

    #[test]
    fn preserves_punctuation_elisions_and_newlines() {
        let d = dict(&["Aquadrox", "API", "serde"]);
        assert_eq!(
            apply("d’aquatroc,\nl'api (et cerde) !", &d, &[]),
            "d’Aquadrox,\nl'API (et serde) !"
        );
        // Majuscule en début de phrase conservée pour une entrée en minuscules.
        assert_eq!(
            apply("Cerde fonctionne bien avec serde.", &d, &[]),
            "Serde fonctionne bien avec serde."
        );
    }

    #[test]
    fn anti_false_positive_guards_protect_common_french_words_and_short_tokens() {
        let d = dict(&["Tauri", "Rust", "Vosk", "Claude", "API", "CI", "SI", "EN"]);
        let sentences = [
            "J'ai tort de partir si tard pour la tour et le tarif.",
            "Il reste juste un peu de temps, c'est rude mais sans ruse.",
            "Vos voix comptent en code si chaud que la cause échappe.",
            "Mon ami a agi ci-joint en silence, ou presque.",
            "En effet, si cela marche ici, on verra demain.",
        ];
        for s in sentences {
            assert_eq!(
                apply(s, &d, &[]),
                s,
                "faux positif détecté sur la phrase : {s}"
            );
        }
        // Un mot-outil de 2 lettres déjà en majuscules est respecté.
        assert_eq!(
            apply("La CI passe sur l'API.", &d, &[]),
            "La CI passe sur l'API."
        );
    }

    #[test]
    fn ambiguity_guard_keeps_original_when_two_dictionary_entries_tie() {
        // « clode » est à même distance phonétique (0) et d'édition (2) de « Claude » et « Clode »?
        // Testons un vrai ex-aequo : « Marceau » et « Marsaux » pour « marso ».
        let d = dict(&["Marceau", "Marsaux"]);
        let out = apply("Bonjour marso", &d, &[]);
        // Les deux entrées ont exactement la même clé phonétique « marso » et
        // la même distance d'édition (3) -> garde d'ambiguïté, texte inchangé.
        assert_eq!(out, "Bonjour marso");
    }

    #[test]
    fn multi_word_dictionary_and_hyphenated_terms_match() {
        let d = dict(&["Visual Studio", "Wi-Fi", "Jean-Luc", "Node.js"]);
        assert_eq!(
            apply(
                "jean luc ouvre visual studio sur le wi fi avec node js.",
                &d,
                &[]
            ),
            "Jean-Luc ouvre Visual Studio sur le Wi-Fi avec Node.js."
        );
    }
}
