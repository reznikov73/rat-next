//! Спайк P0-Б: минимальный UI вьюера на iced.
//! Панель устройств (слева), область видео-заглушки (центр), строка статуса (снизу).
//!
//! Smoke-режим: `cargo run -- --smoke` — окно закрывается само через ~4 секунды.

use std::time::{Duration, Instant};

use iced::widget::{button, column, container, row, rule, scrollable, text};
use iced::{Element, Length, Task};

fn main() -> iced::Result {
    let smoke = std::env::args().any(|a| a == "--smoke");
    println!("UI_ICED_STARTING smoke={smoke}");

    iced::application(
        move || ViewerState::new(smoke),
        ViewerState::update,
        ViewerState::view,
    )
    .title("rat-viewer-iced-spike")
    .window_size(iced::Size::new(1100.0, 700.0))
    .subscription(ViewerState::subscription)
    .run()?;

    println!("UI_ICED_SMOKE_OK");
    Ok(())
}

#[derive(Debug, Clone)]
enum Message {
    Select(usize),
    Tick,
}

struct ViewerState {
    devices: Vec<(String, String)>,
    selected: usize,
    smoke: bool,
    started: Instant,
}

impl ViewerState {
    fn new(smoke: bool) -> Self {
        Self {
            devices: vec![
                ("000123".into(), "Касса №1 (магазин А)".into()),
                ("000456".into(), "Касса №2 (магазин А)".into()),
                ("000789".into(), "Бухгалтерия-ПК".into()),
                ("000321".into(), "Сервер-склад".into()),
            ],
            selected: 0,
            smoke,
            started: Instant::now(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Select(i) => self.selected = i,
            Message::Tick => {}
        }

        if self.smoke && self.started.elapsed() >= Duration::from_secs(4) {
            return iced::exit();
        }
        Task::none()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        let devices =
            self.devices
                .iter()
                .enumerate()
                .fold(column![].spacing(4), |col, (i, (id, desc))| {
                    col.push(
                        button(text(format!("{id}  {desc}")))
                            .width(Length::Fill)
                            .on_press(Message::Select(i)),
                    )
                });

        let sidebar = container(
            column![
                text("Устройства").size(20),
                rule::horizontal(1),
                scrollable(devices)
            ]
            .spacing(8)
            .padding(10),
        )
        .width(Length::Fixed(260.0))
        .height(Length::Fill);

        let (id, desc) = &self.devices[self.selected];
        let video = container(text("VIDEO PLACEHOLDER").size(26))
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill);

        let main_area = column![text(format!("Экран: {id} — {desc}")).size(18), video]
            .spacing(8)
            .padding(10)
            .width(Length::Fill);

        let status = container(
            row![
                text("● не подключено"),
                text("латентность: —"),
                if self.smoke {
                    text("smoke-режим")
                } else {
                    text("интерактив")
                },
            ]
            .spacing(20),
        )
        .padding(8)
        .width(Length::Fill);

        column![
            row![sidebar, main_area].height(Length::Fill),
            rule::horizontal(1),
            status
        ]
        .into()
    }
}
