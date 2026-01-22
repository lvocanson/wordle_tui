use std::io;
use wordle_solver::*;

fn main() {
    let word_length = 5;
    let max_guesses = 6;
    let words_pool = include_str!("words_pool.txt");
    let words_pool: Vec<&str> = words_pool
        .split(|c: char| !c.is_alphabetic())
        .collect();

    let words_dictionary = include_str!("words_dictionary.txt");
    let words_dictionary: Vec<&str> = words_dictionary
        .split(|c: char| !c.is_alphabetic())
        .collect();

    let game = GameBuilder::new(word_length, max_guesses)
        .add_words_to_pool(&words_pool)
        .add_words_to_dictionary(&words_dictionary)
        .build();

    drop(words_pool);
    drop(words_dictionary);

    let mut game = match game {
        Ok(game) => game,
        Err(e) => {
            eprintln!("An error occurred while setting up the game: {e:?}");
            return;
        }
    };

    while matches!(game.state(), GameState::Playing) {
        let (used, max) = game.guesses();
        println!("({used}/{max}) Enter a {word_length} letter word:");

        let mut guess = String::new();
        io::stdin()
            .read_line(&mut guess)
            .expect("Failed to read line");

        let guess = guess.trim();
        let result = game.guess(guess);

        match result {
            GuessResult::NotAllowed => {
                println!("This word is not allowed. (Wrong size or not in the dictionary)");
            }
            GuessResult::Correct => {
                print!("\x1b[42;1m");
                guess.chars().for_each(|c| print!(" {c} "));
                println!("\x1b[0m");
            }
            GuessResult::Incorrect(hints) => {
                for (i, c) in guess.chars().enumerate() {
                    let color_code = match hints[i] {
                        CharHint::Correct => "\x1b[42;1m",
                        CharHint::Misplaced => "\x1b[43;1m",
                        CharHint::NotPresent => "\x1b[100;1m",
                    };
                    print!("{} {} \x1b[0m", color_code, c);
                }
                println!();
            }
        }
    }

    match game.state() {
        GameState::Playing => panic!("Game ended to soon."),
        GameState::Won => {
            let (used, max) = game.guesses();
            println!("Congrats! Found in {used}/{max} guesses.");
        }
        GameState::Lost => {
            let answer = game.get_answer().expect("Unable to retrieve answer from lost game.");
            println!("The word was {answer}.")
        }
    }

    println!("Press Enter to continue...");
    let mut _s = String::new();
    _ = io::stdin().read_line(&mut _s);
}
