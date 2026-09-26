//! This contains the functions to use for rendering the emoji page
use iced::{Border, Length::Fill, border::Radius, widget::tooltip};

use crate::{
    app::pages::prelude::*,
    clipboard::ClipBoardContentType,
    commands::Function,
    styles::{PRIMARY, label, rim, window_fill, with_alpha},
};

/// The emoji pages element to render
///
/// Takes:
/// - the [`Theme`]
/// - the emojis to render
/// - the focussed id
pub fn emoji_page(
    tile_theme: Theme,
    emojis: Vec<App>,
    focussed_id: u32,
) -> Element<'static, Message> {
    let emoji_vec = emojis
        .chunks(6)
        .map(|x| x.to_vec())
        .collect::<Vec<Vec<App>>>();

    let mut column = Vec::new();

    let mut id_num = 0;

    for emoji_row in emoji_vec {
        let mut emoji_row_element = Row::new().spacing(10);
        for emoji in emoji_row {
            let theme_clone = tile_theme.clone();

            // Emoji text
            let element_column = Column::new().push(
                Text::new(emoji.display_name.clone())
                    .font(tile_theme.font())
                    .size(30)
                    .width(Length::Fill)
                    .height(Fill)
                    .align_y(Alignment::Center)
                    .align_x(Alignment::Center),
            );
            let value = tile_theme.clone();
            let value_two = tile_theme.clone();

            // Emoji icon + Emoji container
            emoji_row_element = emoji_row_element.push(tooltip(
                container(
                    Button::new(element_column)
                        .width(70)
                        .height(70)
                        .on_press(Message::RunFunction(Function::CopyToClipboard(
                            ClipBoardContentType::Text(emoji.display_name),
                        )))
                        .style(move |_, status| emoji_button_style(&value, status)),
                )
                .width(70)
                .height(70)
                .id(format!("result-{}", id_num))
                .style(move |_| emoji_button_container_style(&theme_clone, focussed_id == id_num)),
                container(
                    Text::new(emoji.desc)
                        .font(tile_theme.font())
                        .size(12)
                        .color(label(&tile_theme, PRIMARY)),
                )
                .padding([4, 8])
                .style(move |_| container::Style {
                    // Tooltips are solid so they read over the emoji grid.
                    background: Some(Background::Color(with_alpha(window_fill(&value_two), 1.0))),
                    border: Border {
                        color: rim(&value_two),
                        width: 1.0,
                        radius: Radius::new(7.0),
                    },
                    shadow: iced::Shadow {
                        color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.25),
                        offset: iced::Vector::new(0.0, 3.0),
                        blur_radius: 10.0,
                    },
                    ..Default::default()
                }),
                tooltip::Position::Top,
            ));

            id_num += 1;
        }

        column.push(container(emoji_row_element).center_y(70).into());
    }

    container(Column::from_vec(column).spacing(10))
        .padding(10)
        .center_x(WINDOW_WIDTH)
        .into()
}
