use std::error::Error;
use unicode_segmentation::UnicodeSegmentation;

use termion::event::Key;

use crate::{
    config::{self, Suggestor},
    formatter::{StringFormatter, DEFAULT_STRING_FORMATTER},
    renderer::Renderer,
    terminal::Terminal,
    utils::paginate,
    validator::StringValidator,
    OptionAnswer,
};

const DEFAULT_HELP_MESSAGE: &str = "↑↓ to move, tab to auto-complete, enter to submit";

#[derive(Clone)]
pub struct Text<'a> {
    pub message: &'a str,
    pub default: Option<&'a str>,
    pub help_message: Option<&'a str>,
    pub formatter: StringFormatter,
    pub validator: Option<StringValidator>,
    pub page_size: usize,
    pub suggestor: Option<Suggestor>,
}

impl<'a> Text<'a> {
    pub const DEFAULT_PAGE_SIZE: usize = config::DEFAULT_PAGE_SIZE;
    pub const DEFAULT_FORMATTER: StringFormatter = DEFAULT_STRING_FORMATTER;

    pub fn new(message: &'a str) -> Self {
        Self {
            message,
            default: None,
            help_message: None,
            validator: None,
            formatter: Self::DEFAULT_FORMATTER,
            page_size: Self::DEFAULT_PAGE_SIZE,
            suggestor: None,
        }
    }

    pub fn with_help_message(mut self, message: &'a str) -> Self {
        self.help_message = Some(message);
        self
    }

    pub fn with_default(mut self, message: &'a str) -> Self {
        self.default = Some(message);
        self
    }

    pub fn with_suggestor(mut self, suggestor: Suggestor) -> Self {
        self.suggestor = Some(suggestor);
        self
    }

    pub fn with_formatter(mut self, formatter: StringFormatter) -> Self {
        self.formatter = formatter;
        self
    }

    pub fn with_validator(mut self, validator: StringValidator) -> Self {
        self.validator = Some(validator);
        self
    }

    pub fn prompt(self) -> Result<String, Box<dyn Error>> {
        let terminal = Terminal::new()?;
        let mut renderer = Renderer::new(terminal)?;
        self.prompt_with_renderer(&mut renderer)
    }

    pub(in crate) fn prompt_with_renderer(
        self,
        renderer: &mut Renderer,
    ) -> Result<String, Box<dyn Error>> {
        TextPrompt::from(self).prompt(renderer)
    }
}
pub trait PromptMany {
    fn prompt(self) -> Result<Vec<String>, Box<dyn Error>>;
}

impl<'a, I> PromptMany for I
where
    I: Iterator<Item = Text<'a>>,
{
    fn prompt(self) -> Result<Vec<String>, Box<dyn Error>> {
        self.map(Text::prompt).collect()
    }
}

struct TextPrompt<'a> {
    message: &'a str,
    default: Option<&'a str>,
    help_message: Option<&'a str>,
    content: String,
    formatter: StringFormatter,
    validator: Option<StringValidator>,
    error: Option<String>,
    suggestor: Option<Suggestor>,
    suggested_options: Vec<String>,
    cursor_index: usize,
    page_size: usize,
}

impl<'a> From<Text<'a>> for TextPrompt<'a> {
    fn from(so: Text<'a>) -> Self {
        Self {
            message: so.message,
            default: so.default,
            help_message: so.help_message,
            formatter: so.formatter,
            validator: so.validator,
            suggestor: so.suggestor,
            content: String::new(),
            error: None,
            cursor_index: 0,
            page_size: so.page_size,
            suggested_options: match so.suggestor {
                Some(s) => s(""),
                None => vec![],
            },
        }
    }
}

impl<'a> From<&'a str> for Text<'a> {
    fn from(val: &'a str) -> Self {
        Text::new(val)
    }
}

impl<'a> TextPrompt<'a> {
    fn update_suggestions(&mut self) {
        match self.suggestor {
            Some(suggestor) => {
                self.suggested_options = suggestor(&self.content);
                if self.suggested_options.len() > 0
                    && self.suggested_options.len() <= self.cursor_index
                {
                    self.cursor_index = self.suggested_options.len().saturating_sub(1);
                }
            }
            None => {}
        }
    }

    fn move_cursor_up(&mut self) {
        self.cursor_index = self
            .cursor_index
            .checked_sub(1)
            .or(self.suggested_options.len().checked_sub(1))
            .unwrap_or_else(|| 0);
    }

    fn move_cursor_down(&mut self) {
        self.cursor_index = self.cursor_index.saturating_add(1);
        if self.cursor_index >= self.suggested_options.len() {
            self.cursor_index = 0;
        }
    }

    fn on_change(&mut self, key: Key) {
        let mut dirty = false;

        match key {
            Key::Backspace => {
                let len = self.content[..].graphemes(true).count();
                let new_len = len.saturating_sub(1);
                self.content = self.content[..].graphemes(true).take(new_len).collect();
                dirty = true;
            }
            Key::Up => self.move_cursor_up(),
            Key::Down => self.move_cursor_down(),
            Key::Char('\x17') | Key::Char('\x18') => {
                self.content.clear();
                dirty = true;
            }
            Key::Char(c) => {
                self.content.push(c);
                dirty = true;
            }
            _ => {}
        }

        if dirty {
            self.update_suggestions();
        }
    }

    fn use_select_option(&mut self) {
        let selected_suggestion = self.suggested_options.get(self.cursor_index);

        if let Some(ans) = selected_suggestion {
            self.content = ans.clone();
            self.update_suggestions();
        }
    }

    fn get_final_answer(&self) -> Result<String, String> {
        if self.content.is_empty() {
            match self.default {
                Some(val) => return Ok(val.to_string()),
                None => {}
            }
        }

        if let Some(validator) = self.validator {
            match validator(&self.content) {
                Ok(_) => {}
                Err(err) => return Err(err.to_string()),
            }
        }

        Ok(self.content.clone())
    }

    fn render(&mut self, renderer: &mut Renderer) -> Result<(), std::io::Error> {
        let prompt = &self.message;

        renderer.reset_prompt()?;

        if let Some(err) = &self.error {
            renderer.print_error_message(err)?;
        }

        renderer.print_prompt(&prompt, self.default, Some(&self.content))?;

        let choices = self
            .suggested_options
            .iter()
            .enumerate()
            .map(|(i, val)| OptionAnswer::new(i, val))
            .collect::<Vec<OptionAnswer>>();

        let (paginated_opts, rel_sel) = paginate(self.page_size, &choices, self.cursor_index);
        for (idx, opt) in paginated_opts.iter().enumerate() {
            renderer.print_option(rel_sel == idx, &opt.value)?;
        }

        if let Some(message) = self.help_message {
            renderer.print_help(message)?;
        } else if !choices.is_empty() {
            renderer.print_help(DEFAULT_HELP_MESSAGE)?;
        }

        renderer.flush()?;

        Ok(())
    }

    fn prompt(mut self, renderer: &mut Renderer) -> Result<String, Box<dyn Error>> {
        let final_answer: String;

        loop {
            self.render(renderer)?;

            let key = renderer.read_key()?;

            match key {
                Key::Ctrl('c') => bail!("Input interrupted by ctrl-c"),
                Key::Char('\t') => self.use_select_option(),
                Key::Char('\n') | Key::Char('\r') => match self.get_final_answer() {
                    Ok(answer) => {
                        final_answer = answer;
                        break;
                    }
                    Err(err) => self.error = Some(err),
                },
                key => self.on_change(key),
            }
        }

        renderer.cleanup(&self.message, (self.formatter)(&final_answer))?;

        Ok(final_answer)
    }
}

#[cfg(test)]
mod test {
    use ntest::timeout;

    use crate::{renderer::Renderer, terminal::Terminal};

    use super::Text;

    fn default<'a>() -> Text<'a> {
        Text::new("Question?")
    }

    macro_rules! text_test {
        ($name:ident,$input:expr,$output:expr) => {
            text_test! {$name, $input, $output, default()}
        };

        ($name:ident,$input:expr,$output:expr,$prompt:expr) => {
            #[test]
            #[timeout(100)]
            fn $name() {
                let mut read: &[u8] = $input.as_bytes();

                let mut write: Vec<u8> = Vec::new();
                let terminal = Terminal::new_with_io(&mut write, &mut read).unwrap();
                let mut renderer = Renderer::new(terminal).unwrap();

                let ans = $prompt.prompt_with_renderer(&mut renderer).unwrap();

                assert_eq!($output, ans);
            }
        };
    }

    text_test!(empty, "\n", "");

    text_test!(single_letter, "b\n", "b");

    text_test!(letters_and_enter, "normal input\n", "normal input");

    text_test!(
        letters_and_enter_with_emoji,
        "with emoji 🧘🏻‍♂️, 🌍, 🍞, 🚗, 📞\n",
        "with emoji 🧘🏻‍♂️, 🌍, 🍞, 🚗, 📞"
    );

    text_test!(
        input_and_correction,
        "anor\x7F\x7F\x7F\x7Fnormal input\n",
        "normal input"
    );

    text_test!(
        input_and_excessive_correction,
        "anor\x7F\x7F\x7F\x7F\x7F\x7F\x7F\x7Fnormal input\n",
        "normal input"
    );

    text_test!(
        input_correction_after_validation,
        "1234567890\n\x7F\x7F\x7F\x7F\x7F\nyes\n",
        "12345yes",
        Text::new("").with_validator(|ans| match ans.len() {
            len if len > 5 && len < 10 => Ok(()),
            _ => Err("Invalid"),
        })
    );
}
