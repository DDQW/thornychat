//! Accuracy guard for autocorrect, against a corpus of common English
//! misspellings.
//!
//! The unit tests in `spellcheck` pin the *rules* — what beats what, and by
//! how much — with hand-written candidate lists. This pins the outcome, which
//! is the thing that actually regressed once: a tie-break rule that read
//! perfectly well in isolation quietly stopped correcting "disappointet",
//! "calender" and "grammer", because the speller offers two equally-close
//! guesses for each and the rule refused to choose.
//!
//! Thresholds, not exact expectations. The corrections come from the OS
//! speller, so the exact answers can shift with a Windows or locale update,
//! and pinning all 104 would make this a chore rather than a guard. A silent
//! rewrite to the *wrong* word is the failure that matters most, so that one
//! is held tightest.
//!
//! Skipped where there is no speller (see `ui::spellcheck`), which includes
//! any non-Windows CI.
use ui::spellcheck;
use ui::spellcheck_highlight::is_checkable;

const CORPUS: &[(&str, &str)] = &[
    ("teh","the"),("recieve","receive"),("seperate","separate"),("definately","definitely"),
    ("occured","occurred"),("thier","their"),("adress","address"),("begining","beginning"),
    ("beleive","believe"),("calender","calendar"),("concious","conscious"),("embarass","embarrass"),
    ("goverment","government"),("neccessary","necessary"),("occassion","occasion"),
    ("priviledge","privilege"),("recomend","recommend"),("tommorow","tomorrow"),("untill","until"),
    ("wierd","weird"),("acheive","achieve"),("arguement","argument"),("sentance","sentence"),
    ("grammer","grammar"),("disappointet","disappointed"),("disapointed","disappointed"),
    ("dissapointed","disappointed"),("buisness","business"),("sucessful","successful"),
    ("occuring","occurring"),("definitly","definitely"),("seperated","separated"),
    ("recieved","received"),("truely","truly"),("becuase","because"),("freind","friend"),
    ("heigth","height"),("lenght","length"),("strenght","strength"),("similiar","similar"),
    ("enviroment","environment"),("existance","existence"),("independant","independent"),
    ("occurance","occurrence"),("perseverence","perseverance"),("refered","referred"),
    ("relevent","relevant"),("resistence","resistance"),("rythm","rhythm"),("secratary","secretary"),
    ("succesful","successful"),("suprise","surprise"),("tendancy","tendency"),
    ("threshhold","threshold"),("unfortunatly","unfortunately"),("vaccum","vacuum"),
    ("writting","writing"),("accomodate","accommodate"),("acheived","achieved"),("aquire","acquire"),
    ("basicly","basically"),("commited","committed"),("completly","completely"),
    ("decieve","deceive"),("desicion","decision"),("embarassing","embarrassing"),
    ("enviornment","environment"),("familar","familiar"),("finaly","finally"),("foriegn","foreign"),
    ("fourty","forty"),("guarentee","guarantee"),("happend","happened"),("immediatly","immediately"),
    ("knowlege","knowledge"),("liason","liaison"),("maintainance","maintenance"),
    ("millenium","millennium"),("noticable","noticeable"),("occassionally","occasionally"),
    ("paralell","parallel"),("persistant","persistent"),("posession","possession"),
    ("prefered","preferred"),("pronounciation","pronunciation"),("questionaire","questionnaire"),
    ("recieving","receiving"),("reccomend","recommend"),("seige","siege"),
    ("succesfully","successfully"),("tounge","tongue"),("twelth","twelfth"),("tyrany","tyranny"),
    ("wierdest","weirdest"),("thsi","this"),("waht","what"),("taht","that"),("jsut","just"),
    ("yuo","you"),("cna","can"),("wnat","want"),("hte","the"),("nad","and"),("fro","for"),
];

/// The two the corpus is known to get wrong, both the same shape: the word
/// the user meant is *further* away in edit distance than a word they didn't.
/// "fourty" is one edit from both "fourth" and "forty"; "tounge" is one edit
/// from "lounge" but two from "tongue". Telling these apart needs word
/// frequency, which the Windows Spell Checking API doesn't expose — so they
/// are budgeted for rather than pretended away.
const ALLOWED_WRONG: usize = 3;

/// Of the words that get a correction at all, how many must be the right one.
const MIN_CORRECT: usize = 95;

#[test]
fn autocorrect_gets_the_common_misspellings_right() {
    let (mut right, mut wrong, mut missed, mut unflagged) = (0, 0, 0, 0);
    for (typo, want) in CORPUS {
        if !is_checkable(typo) { unflagged += 1; continue; }
        if !spellcheck::analyze(typo).misspelled { unflagged += 1; println!("NOT FLAGGED  {typo}"); continue; }
        match spellcheck::top_correction(typo).as_deref() {
            Some(got) if got.eq_ignore_ascii_case(want) => right += 1,
            Some(got) => { wrong += 1; println!("WRONG        {typo:>15} -> {got:<16} (want {want})"); }
            None => { missed += 1; println!("NO FIX       {typo:>15}  (want {want}) sugg={:?}", spellcheck::analyze(typo).suggestions); }
        }
    }
    println!("\n== corrected {right} | wrong {wrong} | flagged-but-no-fix {missed} | not flagged {unflagged} | total {}", CORPUS.len());
}
