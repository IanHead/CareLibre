//! Name generation for the demo seed.
//!
//! Names are drawn from culturally coherent pools (a given name and surname
//! from the same background) that shift with birth year, so an aged-care client
//! born in 1938 reads as "Doreen Fletcher" or "Giuseppe Russo" and a young
//! participant as "Harper Nguyen". A [`NameBook`] remembers every full name it
//! has issued and retries on a clash, only occasionally letting a coincidental
//! duplicate through, as happens in a real client list.

use std::collections::HashMap;

use rand::{rngs::StdRng, Rng};

/// Chance that a clashing full name is accepted instead of retried.
const DUPLICATE_ACCEPTANCE: f64 = 0.03;
const MAX_ATTEMPTS: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gender {
    Female,
    Male,
}

impl Gender {
    pub fn random(rng: &mut StdRng, female_share: f64) -> Self {
        if rng.gen_bool(female_share) {
            Self::Female
        } else {
            Self::Male
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::Female => Self::Male,
            Self::Male => Self::Female,
        }
    }

    pub const fn pronouns(self) -> &'static str {
        match self {
            Self::Female => "she/her",
            Self::Male => "he/him",
        }
    }

    pub const fn pick(self, female: &'static str, male: &'static str) -> &'static str {
        match self {
            Self::Female => female,
            Self::Male => male,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    Anglo,
    Italian,
    Greek,
    Vietnamese,
    Chinese,
    Indian,
    Arabic,
    Filipino,
}

/// Which population a person is drawn from; each has its own mix of
/// backgrounds, reflecting Melbourne's northern suburbs.
#[derive(Clone, Copy, Debug)]
pub enum Cohort {
    /// Older residents (aged-care clients and their generation).
    Elder,
    /// Everyone else in the community.
    General,
    /// The care workforce.
    Workforce,
}

impl Background {
    pub fn random(rng: &mut StdRng, cohort: Cohort) -> Self {
        use Background::{Anglo, Arabic, Chinese, Filipino, Greek, Indian, Italian, Vietnamese};
        let weights: [(Self, u32); 8] = match cohort {
            Cohort::Elder => [
                (Anglo, 55),
                (Italian, 16),
                (Greek, 14),
                (Vietnamese, 3),
                (Chinese, 4),
                (Indian, 2),
                (Arabic, 4),
                (Filipino, 2),
            ],
            Cohort::General => [
                (Anglo, 50),
                (Italian, 7),
                (Greek, 6),
                (Vietnamese, 7),
                (Chinese, 9),
                (Indian, 11),
                (Arabic, 6),
                (Filipino, 4),
            ],
            Cohort::Workforce => [
                (Anglo, 40),
                (Italian, 6),
                (Greek, 6),
                (Vietnamese, 6),
                (Chinese, 6),
                (Indian, 20),
                (Arabic, 6),
                (Filipino, 10),
            ],
        };
        let total: u32 = weights.iter().map(|(_, weight)| weight).sum();
        let mut roll = rng.gen_range(0..total);
        for (background, weight) in weights {
            if roll < weight {
                return background;
            }
            roll -= weight;
        }
        Anglo
    }

    const fn surnames(self) -> &'static [&'static str] {
        match self {
            Self::Anglo => ANGLO_SURNAMES,
            Self::Italian => ITALIAN_SURNAMES,
            Self::Greek => GREEK_SURNAMES,
            Self::Vietnamese => VIETNAMESE_SURNAMES,
            Self::Chinese => CHINESE_SURNAMES,
            Self::Indian => INDIAN_SURNAMES,
            Self::Arabic => ARABIC_SURNAMES,
            Self::Filipino => FILIPINO_SURNAMES,
        }
    }

    fn given_names(self, gender: Gender, birth_year: i32) -> &'static [&'static str] {
        let older = birth_year < 1965;
        match (self, gender) {
            (Self::Anglo, Gender::Female) => match birth_year {
                ..=1949 => ANGLO_F_1930,
                1950..=1969 => ANGLO_F_1950,
                1970..=1989 => ANGLO_F_1970,
                _ => ANGLO_F_1990,
            },
            (Self::Anglo, Gender::Male) => match birth_year {
                ..=1949 => ANGLO_M_1930,
                1950..=1969 => ANGLO_M_1950,
                1970..=1989 => ANGLO_M_1970,
                _ => ANGLO_M_1990,
            },
            (Self::Italian, Gender::Female) if older => ITALIAN_F_OLD,
            (Self::Italian, Gender::Female) => ITALIAN_F_YOUNG,
            (Self::Italian, Gender::Male) if older => ITALIAN_M_OLD,
            (Self::Italian, Gender::Male) => ITALIAN_M_YOUNG,
            (Self::Greek, Gender::Female) if older => GREEK_F_OLD,
            (Self::Greek, Gender::Female) => GREEK_F_YOUNG,
            (Self::Greek, Gender::Male) if older => GREEK_M_OLD,
            (Self::Greek, Gender::Male) => GREEK_M_YOUNG,
            (Self::Vietnamese, Gender::Female) => VIETNAMESE_F,
            (Self::Vietnamese, Gender::Male) => VIETNAMESE_M,
            (Self::Chinese, Gender::Female) => CHINESE_F,
            (Self::Chinese, Gender::Male) => CHINESE_M,
            (Self::Indian, Gender::Female) => INDIAN_F,
            (Self::Indian, Gender::Male) => INDIAN_M,
            (Self::Arabic, Gender::Female) => ARABIC_F,
            (Self::Arabic, Gender::Male) => ARABIC_M,
            (Self::Filipino, Gender::Female) => FILIPINO_F,
            (Self::Filipino, Gender::Male) => FILIPINO_M,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Name {
    pub given: String,
    pub family: String,
    pub gender: Gender,
    pub background: Background,
}

impl Name {
    pub fn display(&self) -> String {
        format!("{} {}", self.given, self.family)
    }
}

/// Issues names and keeps duplicate full names rare.
#[derive(Default)]
pub struct NameBook {
    used: HashMap<String, u32>,
}

impl NameBook {
    /// A new name. `family` fixes the surname (for relatives); otherwise one is
    /// drawn from the background's pool, weighted towards common surnames.
    pub fn name(
        &mut self,
        rng: &mut StdRng,
        gender: Gender,
        birth_year: i32,
        background: Background,
        family: Option<&str>,
    ) -> Name {
        let mut candidate = None;
        for _ in 0..MAX_ATTEMPTS {
            let given = given_name(rng, gender, birth_year, background);
            let family = family.map_or_else(
                || weighted_pick(rng, background.surnames()).to_owned(),
                str::to_owned,
            );
            let name = Name {
                given: given.to_owned(),
                family,
                gender,
                background,
            };
            let clash = self.used.contains_key(&name.display());
            candidate = Some(name);
            if !clash || rng.gen_bool(DUPLICATE_ACCEPTANCE) {
                break;
            }
        }
        let name = candidate.expect("at least one attempt");
        *self.used.entry(name.display()).or_default() += 1;
        name
    }

    /// Number of full names issued more than once.
    pub fn duplicated(&self) -> usize {
        self.used.values().filter(|count| **count > 1).count()
    }
}

fn given_name(
    rng: &mut StdRng,
    gender: Gender,
    birth_year: i32,
    background: Background,
) -> &'static str {
    // Second-generation Australians often carry an Anglo given name.
    let anglicised = background != Background::Anglo && birth_year >= 1975 && rng.gen_bool(0.3);
    let pool = if anglicised {
        Background::Anglo.given_names(gender, birth_year)
    } else {
        background.given_names(gender, birth_year)
    };
    pool[rng.gen_range(0..pool.len())]
}

/// Picks from a list ordered most-common first, favouring the head of the list.
fn weighted_pick(rng: &mut StdRng, items: &'static [&'static str]) -> &'static str {
    let weights: Vec<f64> = (0..items.len())
        .map(|rank| 1.0 / (rank as f64 + 8.0))
        .collect();
    let total: f64 = weights.iter().sum();
    let mut roll = rng.gen_range(0.0..total);
    for (item, weight) in items.iter().zip(weights) {
        if roll < weight {
            return item;
        }
        roll -= weight;
    }
    items[items.len() - 1]
}

/// The everyday short form of a given name, if it commonly has one.
pub fn nickname(given: &str) -> Option<&'static str> {
    Some(match given {
        "Margaret" => "Peg",
        "Patricia" => "Pat",
        "Elizabeth" => "Liz",
        "Jennifer" => "Jen",
        "Samantha" => "Sam",
        "Robert" => "Bob",
        "William" => "Bill",
        "Kenneth" => "Ken",
        "Raymond" => "Ray",
        "Ronald" => "Ron",
        "Donald" => "Don",
        "Douglas" => "Doug",
        "Stanley" => "Stan",
        "Edward" => "Ted",
        "Thomas" => "Tom",
        "Christopher" => "Chris",
        "Matthew" => "Matt",
        "Nicholas" => "Nick",
        "Benjamin" => "Ben",
        "Alexander" => "Alex",
        "Daniel" => "Dan",
        "Anthony" => "Tony",
        "Gregory" => "Greg",
        "Stephen" => "Steve",
        "Michael" => "Mike",
        "Konstantinos" => "Con",
        "Dimitrios" => "Jim",
        "Georgios" => "George",
        "Ioannis" => "John",
        "Vasiliki" => "Vicky",
        "Panagiota" => "Toula",
        "Giuseppe" => "Joe",
        "Giuseppina" => "Josie",
        "Francesco" => "Frank",
        "Salvatore" => "Sam",
        "Antonietta" => "Toni",
        "Mohammed" => "Mo",
        _ => return None,
    })
}

const ANGLO_SURNAMES: &[&str] = &[
    "Smith",
    "Jones",
    "Williams",
    "Brown",
    "Wilson",
    "Taylor",
    "Johnson",
    "White",
    "Martin",
    "Anderson",
    "Thompson",
    "Thomas",
    "Walker",
    "Harris",
    "Lee",
    "Ryan",
    "Robinson",
    "Kelly",
    "King",
    "Davis",
    "Wright",
    "Evans",
    "Roberts",
    "Green",
    "Hall",
    "Wood",
    "Jackson",
    "Clarke",
    "Mitchell",
    "Hughes",
    "Edwards",
    "Turner",
    "Hill",
    "Campbell",
    "Cooper",
    "Moore",
    "Young",
    "Allen",
    "Murphy",
    "Scott",
    "Ward",
    "Morris",
    "Baker",
    "Stewart",
    "Watson",
    "Adams",
    "Bell",
    "Collins",
    "Parker",
    "Russell",
    "Phillips",
    "Morgan",
    "Carter",
    "Graham",
    "Cook",
    "Bailey",
    "Shaw",
    "Murray",
    "Kennedy",
    "Simpson",
    "Ellis",
    "Marshall",
    "Gray",
    "Stevens",
    "Chapman",
    "Hunt",
    "Mason",
    "Butler",
    "Fraser",
    "Grant",
    "Hamilton",
    "Reid",
    "Ross",
    "Henderson",
    "Hayes",
    "Hart",
    "Dunn",
    "Webb",
    "Burns",
    "Lawrence",
    "Hogan",
    "Doyle",
    "O'Brien",
    "O'Connor",
    "Sullivan",
    "McDonald",
    "McKenzie",
    "Gibson",
    "Fletcher",
    "Barnes",
    "Holmes",
    "Payne",
    "Lloyd",
    "Pearce",
    "Matthews",
    "Richardson",
    "Harvey",
    "Walsh",
    "Byrne",
    "Quinn",
    "Lynch",
    "Brennan",
    "Dwyer",
    "Fitzgerald",
    "Kearney",
    "Maher",
    "Gallagher",
    "McGrath",
    "Nolan",
    "Brady",
    "Sutherland",
    "Crawford",
    "Ferguson",
    "Kerr",
    "Duncan",
    "Lowe",
];

const ITALIAN_SURNAMES: &[&str] = &[
    "Rossi",
    "Russo",
    "Ferrari",
    "Esposito",
    "Romano",
    "Colombo",
    "Ricci",
    "Marino",
    "Greco",
    "Bruno",
    "Gallo",
    "Conti",
    "De Luca",
    "Costa",
    "Giordano",
    "Mancini",
    "Rizzo",
    "Lombardi",
    "Moretti",
    "Barbieri",
    "Fontana",
    "Santoro",
    "Caruso",
    "Ferraro",
    "Mazza",
    "Rinaldi",
    "Pellegrino",
    "Vitale",
    "Bianchi",
    "Martini",
    "Leone",
    "Longo",
    "Gentile",
    "Messina",
    "Sorrentino",
    "Cirillo",
    "Pagano",
    "Fiore",
    "D'Angelo",
    "Palmieri",
];

const GREEK_SURNAMES: &[&str] = &[
    "Papadopoulos",
    "Georgiou",
    "Nikolaidis",
    "Dimitriou",
    "Konstantinidis",
    "Ioannou",
    "Christodoulou",
    "Papageorgiou",
    "Vlahos",
    "Pappas",
    "Karagiannis",
    "Theodorou",
    "Antoniou",
    "Michaelides",
    "Kyriakou",
    "Stavropoulos",
    "Athanasiou",
    "Economou",
    "Panagiotopoulos",
    "Alexiou",
    "Mavros",
    "Galanis",
    "Kostopoulos",
    "Demetriou",
    "Petrakis",
    "Raptis",
    "Lambrou",
    "Spiropoulos",
    "Zervos",
    "Kalogeropoulos",
];

const VIETNAMESE_SURNAMES: &[&str] = &[
    "Nguyen", "Tran", "Le", "Pham", "Huynh", "Hoang", "Phan", "Vo", "Vu", "Dang", "Bui", "Do",
    "Ho", "Ngo", "Duong", "Ly", "Truong", "Dinh", "Lam", "Mai",
];

const CHINESE_SURNAMES: &[&str] = &[
    "Wang", "Li", "Zhang", "Chen", "Liu", "Wong", "Yang", "Huang", "Zhao", "Chan", "Wu", "Zhou",
    "Lau", "Xu", "Sun", "Cheung", "Ma", "Zhu", "Ng", "Hu", "Guo", "Tan", "He", "Lin", "Gao", "Lim",
    "Luo", "Ho",
];

const INDIAN_SURNAMES: &[&str] = &[
    "Singh", "Patel", "Sharma", "Kumar", "Gupta", "Shah", "Reddy", "Nair", "Gill", "Sandhu",
    "Iyer", "Menon", "Rao", "Joshi", "Mehta", "Dhillon", "Chopra", "Malhotra", "Pillai", "Verma",
    "Desai", "Bhatt", "Kapoor", "Sidhu",
];

const ARABIC_SURNAMES: &[&str] = &[
    "Haddad", "Khoury", "Hassan", "Ahmed", "Ali", "Ibrahim", "Saleh", "Nasser", "Mansour", "Habib",
    "Aziz", "Rahman", "Karam", "Fakhoury", "Abboud", "Chahine", "Hamdan", "Mourad", "Daher",
    "Issa", "El-Masri", "Saad",
];

const FILIPINO_SURNAMES: &[&str] = &[
    "Santos",
    "Reyes",
    "Cruz",
    "Bautista",
    "Garcia",
    "Mendoza",
    "Torres",
    "Flores",
    "Dela Cruz",
    "Ramos",
    "Aquino",
    "Villanueva",
    "Castillo",
    "Rivera",
    "Gonzales",
    "Navarro",
    "Domingo",
    "Fernandez",
    "Salazar",
    "Pascual",
];

const ANGLO_F_1930: &[&str] = &[
    "Margaret", "Patricia", "Joan", "Dorothy", "Shirley", "Betty", "Jean", "Barbara", "Beverley",
    "Valerie", "Joyce", "June", "Marjorie", "Gwen", "Elaine", "Nancy", "Doreen", "Irene", "Mavis",
    "Phyllis", "Lorraine", "Judith", "Helen", "Maureen", "Kathleen", "Audrey", "Noreen", "Edna",
    "Elsie", "Gloria",
];

const ANGLO_M_1930: &[&str] = &[
    "John", "Robert", "William", "Kenneth", "Raymond", "Ronald", "Donald", "Keith", "Graham",
    "Bruce", "Colin", "Barry", "Geoffrey", "Norman", "Alan", "Brian", "Leslie", "Neville", "Allan",
    "Douglas", "Frank", "Arthur", "Harold", "Stanley", "Gordon", "Trevor", "Clifford", "Edward",
    "Thomas", "Ian",
];

const ANGLO_F_1950: &[&str] = &[
    "Susan",
    "Julie",
    "Karen",
    "Debra",
    "Jennifer",
    "Wendy",
    "Christine",
    "Deborah",
    "Lynette",
    "Kerry",
    "Robyn",
    "Sharon",
    "Janet",
    "Gail",
    "Denise",
    "Linda",
    "Jillian",
    "Cheryl",
    "Pamela",
    "Diane",
    "Leanne",
    "Elizabeth",
    "Anne",
    "Gillian",
    "Rosemary",
    "Vicki",
    "Heather",
    "Sandra",
    "Carol",
    "Michelle",
];

const ANGLO_M_1950: &[&str] = &[
    "Peter",
    "David",
    "Michael",
    "Stephen",
    "Paul",
    "Mark",
    "Gregory",
    "Anthony",
    "Andrew",
    "Wayne",
    "Gary",
    "Phillip",
    "Darren",
    "Craig",
    "Glenn",
    "Rodney",
    "Russell",
    "Shane",
    "Kevin",
    "Steven",
    "Brett",
    "Dean",
    "Warren",
    "Grant",
    "Murray",
    "Neil",
    "Gavin",
    "Terry",
    "Christopher",
    "Richard",
];

const ANGLO_F_1970: &[&str] = &[
    "Sarah", "Rebecca", "Melissa", "Kylie", "Nicole", "Lisa", "Amanda", "Rachel", "Kate", "Emma",
    "Jessica", "Natalie", "Belinda", "Danielle", "Fiona", "Renee", "Kristy", "Joanne", "Tanya",
    "Megan", "Alison", "Bianca", "Jodie", "Katrina", "Naomi", "Simone", "Tracey", "Leah",
    "Samantha", "Carly",
];

const ANGLO_M_1970: &[&str] = &[
    "Matthew", "Daniel", "Benjamin", "Luke", "Adam", "Nathan", "Jason", "Scott", "Simon", "Jamie",
    "Aaron", "Bradley", "Justin", "Timothy", "Joshua", "Damien", "Jarrod", "Troy", "Cameron",
    "Ryan", "Nicholas", "Jeremy", "Travis", "Brendan", "Corey", "Hayden", "Mitchell", "Shaun",
    "Jonathan", "Ashley",
];

const ANGLO_F_1990: &[&str] = &[
    "Olivia",
    "Charlotte",
    "Amelia",
    "Isla",
    "Mia",
    "Ava",
    "Grace",
    "Chloe",
    "Ruby",
    "Zoe",
    "Matilda",
    "Sienna",
    "Harper",
    "Ella",
    "Willow",
    "Ivy",
    "Georgia",
    "Lily",
    "Sophie",
    "Emily",
    "Hannah",
    "Madison",
    "Jasmine",
    "Holly",
    "Tahlia",
    "Brooke",
    "Abbey",
    "Lucy",
    "Evie",
    "Scarlett",
];

const ANGLO_M_1990: &[&str] = &[
    "Liam", "Noah", "Oliver", "Jack", "William", "Henry", "Thomas", "Ethan", "Lucas", "Mason",
    "Hudson", "Archer", "Levi", "Hunter", "Jasper", "Lachlan", "Cooper", "Harrison", "Riley",
    "Jayden", "Bailey", "Blake", "Flynn", "Max", "Oscar", "Charlie", "Angus", "Toby", "Kai",
    "Xavier",
];

const ITALIAN_F_OLD: &[&str] = &[
    "Maria",
    "Giuseppina",
    "Rosa",
    "Carmela",
    "Antonietta",
    "Concetta",
    "Lucia",
    "Teresa",
    "Filomena",
    "Assunta",
    "Angela",
    "Rosaria",
    "Pina",
    "Grazia",
    "Nunzia",
];

const ITALIAN_F_YOUNG: &[&str] = &[
    "Laura",
    "Daniela",
    "Sofia",
    "Giulia",
    "Alessandra",
    "Chiara",
    "Isabella",
    "Gabriella",
    "Elena",
    "Silvana",
    "Francesca",
    "Anna",
    "Christina",
    "Valentina",
    "Bianca",
];

const ITALIAN_M_OLD: &[&str] = &[
    "Giuseppe",
    "Antonio",
    "Salvatore",
    "Francesco",
    "Giovanni",
    "Vincenzo",
    "Domenico",
    "Luigi",
    "Mario",
    "Rocco",
    "Carmelo",
    "Pasquale",
    "Angelo",
    "Bruno",
    "Sergio",
];

const ITALIAN_M_YOUNG: &[&str] = &[
    "Marco",
    "Luca",
    "Matteo",
    "Alessandro",
    "Dominic",
    "Lorenzo",
    "Stefano",
    "Adriano",
    "Fabio",
    "Paolo",
    "Joseph",
    "Anthony",
    "Daniel",
    "Vince",
    "Frank",
];

const GREEK_F_OLD: &[&str] = &[
    "Maria",
    "Eleni",
    "Georgia",
    "Vasiliki",
    "Ekaterini",
    "Dimitra",
    "Sophia",
    "Anastasia",
    "Paraskevi",
    "Despina",
    "Chrysoula",
    "Kalliopi",
    "Stavroula",
    "Androula",
    "Panagiota",
];

const GREEK_F_YOUNG: &[&str] = &[
    "Eleni",
    "Sophia",
    "Anastasia",
    "Christina",
    "Despina",
    "Georgia",
    "Alexandra",
    "Katerina",
    "Zoe",
    "Athena",
    "Marina",
    "Irene",
    "Stephanie",
    "Voula",
    "Penelope",
];

const GREEK_M_OLD: &[&str] = &[
    "Georgios",
    "Konstantinos",
    "Ioannis",
    "Dimitrios",
    "Nikolaos",
    "Panagiotis",
    "Vasilios",
    "Christos",
    "Athanasios",
    "Stavros",
    "Spiros",
    "Theodoros",
    "Michail",
    "Evangelos",
    "Kostas",
];

const GREEK_M_YOUNG: &[&str] = &[
    "Nicholas",
    "Alexander",
    "Dimitri",
    "Theo",
    "Yianni",
    "Stefan",
    "George",
    "Peter",
    "Chris",
    "Jason",
    "Andreas",
    "Leo",
    "Steven",
    "Con",
    "Nektarios",
];

const VIETNAMESE_F: &[&str] = &[
    "Linh", "Lan", "Huong", "Mai", "Thu", "Hoa", "Ngoc", "Phuong", "Trang", "Hanh", "Thuy", "Anh",
    "Hien", "Nga", "Tuyet", "Diep", "Kim", "Loan", "Van", "Yen",
];

const VIETNAMESE_M: &[&str] = &[
    "Minh", "Tuan", "Hung", "Duc", "Quang", "Thanh", "Hai", "Long", "Nam", "Phuc", "Dung", "Hieu",
    "Khanh", "Son", "Tam", "Vinh", "Bao", "Huy", "Trung", "Cuong",
];

const CHINESE_F: &[&str] = &[
    "Mei", "Xin", "Jing", "Yan", "Hui", "Ling", "Ying", "Fang", "Xiu", "Lan", "Hong", "Yu", "Qing",
    "Jia", "Shu", "Min", "Ting", "Yun", "Xiaoling", "Lihua",
];

const CHINESE_M: &[&str] = &[
    "Wei", "Jun", "Hao", "Ming", "Jian", "Lei", "Tao", "Yong", "Feng", "Bo", "Gang", "Kai",
    "Liang", "Qiang", "Bin", "Hua", "Zhi", "Peng", "Jiahao", "Zihan",
];

const INDIAN_F: &[&str] = &[
    "Priya", "Anjali", "Pooja", "Neha", "Kavita", "Sunita", "Deepa", "Meera", "Lakshmi", "Asha",
    "Divya", "Shreya", "Ananya", "Kiran", "Harpreet", "Gurpreet", "Simran", "Nisha", "Rekha",
    "Swati",
];

const INDIAN_M: &[&str] = &[
    "Raj", "Amit", "Rahul", "Sanjay", "Vikram", "Arjun", "Rohan", "Anil", "Suresh", "Ravi",
    "Harjit", "Gurdeep", "Manpreet", "Aarav", "Karan", "Nikhil", "Deepak", "Pradeep", "Vivek",
    "Aditya",
];

const ARABIC_F: &[&str] = &[
    "Fatima", "Mariam", "Layla", "Zainab", "Amira", "Nadia", "Rania", "Huda", "Samira", "Leila",
    "Yasmin", "Hana", "Mona", "Salma", "Noor", "Rana", "Dalia", "Jana", "Sara", "Lina",
];

const ARABIC_M: &[&str] = &[
    "Mohammed", "Ahmed", "Ali", "Omar", "Hassan", "Khaled", "Youssef", "Ibrahim", "Mahmoud",
    "Tariq", "Karim", "Sami", "Bilal", "Ziad", "Rami", "Hussein", "Adam", "Jamal", "Nabil", "Fadi",
];

const FILIPINO_F: &[&str] = &[
    "Maria",
    "Ana",
    "Cristina",
    "Rosario",
    "Marites",
    "Jocelyn",
    "Lorna",
    "Gemma",
    "Rowena",
    "Analyn",
    "Joy",
    "Grace",
    "Angelica",
    "Kristine",
    "Precious",
    "Mylene",
    "Rhea",
    "Charmaine",
    "Liza",
    "Arlene",
];

const FILIPINO_M: &[&str] = &[
    "Jose",
    "Juan",
    "Ramon",
    "Rodel",
    "Ricardo",
    "Arnel",
    "Jerome",
    "Mark",
    "Paolo",
    "Christian",
    "Rommel",
    "Joel",
    "Jun",
    "Danilo",
    "Eduardo",
    "Renato",
    "Carlo",
    "Jayson",
    "Noel",
    "Ferdinand",
];
