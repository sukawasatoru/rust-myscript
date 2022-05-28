struct OsInfo(&'static [&'static str], &'static str);

fn main() {
    let mut infos = vec![
        OsInfo(
            &["b", "1.0", "1", "api1"],
            r#"The original version
  Initial release: 2008
  VERSION_CODES: BASE
  API Level: 1
"#,
        ),
        OsInfo(
            &["b", "1.1", "1", "api2", "2"],
            r#"Android 1.1
  Initial release: 2009
  VERSION_CODES: BASE_1_1
  API Level: 2
"#,
        ),
        OsInfo(
            &["c", "1.5", "1", "cupcake", "cup", "cake", "api3", "3"],
            r#"Android 1.5
  Initial release: 2009
  Code name: Cupcake
  VERSION_CODES: CUPCAKE
  API Level: 3
"#,
        ),
        OsInfo(
            &["d", "1.6", "1", "donut", "api4", "4"],
            r#"Android 1.6
  Initial release: 2009
  Code name: Donut
  VERSION_CODES: DONUT
  API Level: 4
"#,
        ),
        OsInfo(
            &["e", "2.0", "2", "api5", "5"],
            r#"Android 2.0
  Initial release: 2009
  Code name: Eclair
  VERSION_CODES: ECLAIR
  API Level: 5
"#,
        ),
        OsInfo(
            &["e", "2.0.1", "2", "api6", "6"],
            r#"Android 2.0.1
  Initial release: 2009
  Code name: Eclair
  VERSION_CODES: ECLAIR_0_1
  API Level: 6
"#,
        ),
        OsInfo(
            &["e", "2.1", "2", "api7", "7"],
            r#"Android 2.1
  Initial release: 2010
  Code name: Eclair
  VERSION_CODES: ECLAIR_MR1
  API Level: 7
"#,
        ),
        OsInfo(
            &["f", "2.2", "2", "froyo", "api8", "8"],
            r#"Android 2.2
  Initial release: 2010
  Code name: Eclair
  VERSION_CODES: FROYO
  API Level: 8
"#,
        ),
        OsInfo(
            &[
                "g",
                "2.3",
                "2",
                "gingerbread",
                "ginger",
                "bread",
                "api9",
                "9",
            ],
            r#"Android 2.3
  Initial release: 2010
  Code name: Gingerbread
  VERSION_CODES: GINGERBREAD
  API Level: 9
"#,
        ),
        OsInfo(
            &[
                "g",
                "2.3.3",
                "2",
                "gingerbread",
                "ginger",
                "bread",
                "api10",
                "10",
            ],
            r#"Android 2.3.3
  Initial release: 2011
  Code name: Gingerbread
  VERSION_CODES: GINGERBREAD_MR1
  API Level: 10
"#,
        ),
        OsInfo(
            &["h", "3.0", "3", "honeycomb", "honey", "comb", "api11", "11"],
            r#"Android 3.0
  Initial release: 2011
  Code name: Honeycomb
  VERSION_CODES: HONEYCOMB
  API Level: 11
"#,
        ),
        OsInfo(
            &["h", "3.1", "3", "honeycomb", "honey", "comb", "api12", "12"],
            r#"Android 3.1
  Initial release: 2011
  Code name: Honeycomb
  VERSION_CODES: HONEYCOMB_MR1
  API Level: 12
"#,
        ),
        OsInfo(
            &["h", "3.2", "3", "honeycomb", "honey", "comb", "api13", "13"],
            r#"Android 3.2
  Initial release: 2011
  Code name: Honeycomb
  VERSION_CODES: HONEYCOMB_MR2
  API Level: 13
"#,
        ),
        OsInfo(
            &[
                "i",
                "4.0",
                "4",
                "icecreamsandwich",
                "ice",
                "cream",
                "sandwich",
                "ics",
                "api14",
                "14",
            ],
            r#"Android 4.0
  Initial release: 2011
  Code name: Ice Cream Sandwich
  VERSION_CODES: ICE_CREAM_SANDWICH
  API Level: 14
"#,
        ),
        OsInfo(
            &[
                "i",
                "4.0.3",
                "4",
                "icecreamsandwich",
                "ice",
                "cream",
                "sandwich",
                "ics",
                "api15",
                "15",
            ],
            r#"Android 4.0.3
  Initial release: 2011
  Code name: Ice Cream Sandwich
  VERSION_CODES: ICE_CREAM_SANDWICH_MR1
  API Level: 15
"#,
        ),
        OsInfo(
            &[
                "j",
                "4.1",
                "4",
                "jellybean",
                "jelly",
                "bean",
                "jb",
                "api16",
                "16",
            ],
            r#"Android 4.1
  Initial release: 2012
  Code name: Jelly Bean
  VERSION_CODES: JELLY_BEAN
  API Level: 16
"#,
        ),
        OsInfo(
            &[
                "j",
                "4.2",
                "4",
                "jellybean",
                "jelly",
                "bean",
                "jb",
                "api17",
                "17",
            ],
            r#"Android 4.2
  Initial release: 2012
  Code name: Jelly Bean
  VERSION_CODES: JELLY_BEAN_MR1
  API Level: 17
"#,
        ),
        OsInfo(
            &[
                "j",
                "4.3",
                "4",
                "jellybean",
                "jelly",
                "bean",
                "jb",
                "api18",
                "18",
            ],
            r#"Android 4.3
  Initial release: 2013
  Code name: Jelly Bean
  VERSION_CODES: JELLY_BEAN_MR2
  API Level: 18
"#,
        ),
        OsInfo(
            &["k", "4.4", "4", "kitkat", "api19", "19"],
            r#"Android 4.4
  Initial release: 2013
  Code name: KitKat
  VERSION_CODES: KITKAT
  API Level: 19
"#,
        ),
        OsInfo(
            &["k", "4.4W", "4", "kitkat", "watch", "api20", "20"],
            r#"Android 4.4W
  Initial release: 2014
  Code name: KitKat
  VERSION_CODES: KITKAT_WATCH
  API Level: 20
"#,
        ),
        OsInfo(
            &["l", "5.0", "5", "lollipop", "lolli", "pop", "api21", "21"],
            r#"Android 5.0
  Initial release: 2014
  Code name: Lollipop
  VERSION_CODES: LOLLIPOP
  API Level: 21
"#,
        ),
        OsInfo(
            &["l", "5.1", "5", "lollipop", "lolli", "pop", "api22", "22"],
            r#"Android 5.1
  Initial release: 2015
  Code name: Lollipop
  VERSION_CODES: LOLLIPOP_MR1
  API Level: 22
"#,
        ),
        OsInfo(
            &[
                "m",
                "6.0",
                "6",
                "marshmallow",
                "marsh",
                "mallow",
                "api23",
                "23",
            ],
            r#"Android 6.0
  Initial release: 2015
  Code name: Marshmallow
  VERSION_CODES: M
  API Level: 23
"#,
        ),
        OsInfo(
            &["n", "7.0", "7", "nougat", "api24", "24"],
            r#"Android 7.0
  Initial release: 2016
  Code name: Nougat
  VERSION_CODES: N
  API Level: 24
"#,
        ),
        OsInfo(
            &["n", "7.1", "7", "nougat", "api25", "25"],
            r#"Android 7.1
  Initial release: 2016
  Code name: Nougat
  VERSION_CODES: N_MR1
  API Level: 25
"#,
        ),
        OsInfo(
            &["o", "8.0", "8", "oreo", "api26", "26"],
            r#"Android 8.0
  Initial release: 2017
  Code name: Oreo
  VERSION_CODES: O
  API Level: 26
"#,
        ),
        OsInfo(
            &["o", "8.1", "8", "oreo", "api27", "27"],
            r#"Android 8.1
  Initial release: 2017
  Code name: Oreo
  VERSION_CODES: O_MR1
  API Level: 27
"#,
        ),
        OsInfo(
            &["p", "9", "pie", "api28", "28"],
            r#"Android 9
  Initial release: 2018
  Code name: Pie
  VERSION_CODES: P
  API Level: 28
"#,
        ),
        OsInfo(
            &["q", "10", "api29", "29"],
            r#"Android 10
  Initial release: 2019
  VERSION_CODES: Q
  API Level: 29
"#,
        ),
        OsInfo(
            &["r", "11", "api30", "30"],
            r#"Android 11
  Initial release: 2020
  VERSION_CODES: R
  API Level: 30
"#,
        ),
        OsInfo(
            &["s", "12", "api31", "31"],
            r#"Android 12
  Initial release: 2021
  VERSION_CODES: S
  API Level: 31
"#,
        ),
        OsInfo(
            &["s", "12", "12l", "api32", "32"],
            r#"Android 12L
  Initial release: 2022
  VERSION_CODES: S
  API Level: 32
"#,
        ),
    ];

    for param in std::env::args().skip(1) {
        let param = param.to_ascii_lowercase();
        let (drained, unmatched) = infos
            .into_iter()
            .partition(|data| data.0.contains(&param.as_str()));
        infos = unmatched;
        for entry in drained {
            print!("{}", entry.1);
        }
    }
}
