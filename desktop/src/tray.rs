//! Windows system-tray integration. The tray is created from the Iced update
//! loop so tray-icon shares the active Windows event-loop thread.
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Open,
    ToggleHost,
    ToggleInput,
    Exit,
}

pub struct Tray {
    // The tray icon is reference-counted by tray-icon. Keeping this value alive
    // is what keeps the notification-area icon registered.
    _icon: TrayIcon,
    open: MenuItem,
    toggle_host: MenuItem,
    toggle_input: MenuItem,
    exit: MenuItem,
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let open = MenuItem::with_id("p2p-open", "打开 P2P Remote", true, None);
        let toggle_host =
            MenuItem::with_id("p2p-toggle-host", "启动/停止本机共享", true, None);
        let toggle_input =
            MenuItem::with_id("p2p-toggle-input", "暂停/恢复远程输入", true, None);
        let exit = MenuItem::with_id("p2p-exit", "完全退出", true, None);

        let menu = Menu::new();
        menu.append(&open).map_err(|error| error.to_string())?;
        menu.append(&toggle_host).map_err(|error| error.to_string())?;
        menu.append(&toggle_input).map_err(|error| error.to_string())?;
        menu.append(&exit).map_err(|error| error.to_string())?;

        let icon = make_icon()?;
        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("P2P Remote")
            .with_menu_on_left_click(false)
            .build()
            .map_err(|error| error.to_string())?;

        Ok(Self {
            _icon: tray_icon,
            open,
            toggle_host,
            toggle_input,
            exit,
        })
    }

    /// Drain tray and menu events on the Iced/UI thread.
    pub fn drain_actions(&self) -> Vec<Action> {
        let mut actions = Vec::new();

        for event in MenuEvent::receiver().try_iter() {
            if event.id == *self.open.id() {
                actions.push(Action::Open);
            } else if event.id == *self.toggle_host.id() {
                actions.push(Action::ToggleHost);
            } else if event.id == *self.toggle_input.id() {
                actions.push(Action::ToggleInput);
            } else if event.id == *self.exit.id() {
                actions.push(Action::Exit);
            }
        }

        for event in TrayIconEvent::receiver().try_iter() {
            match event {
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => actions.push(Action::Open),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => actions.push(Action::Open),
                _ => {}
            }
        }

        actions
    }
}

fn make_icon() -> Result<Icon, String> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            let inside = dx * dx + dy * dy <= 225;
            if !inside {
                continue;
            }

            let offset = ((y * SIZE + x) * 4) as usize;
            let monitor = (9..23).contains(&x) && (9..20).contains(&y);
            let stand = (14..18).contains(&x) && (20..24).contains(&y);

            let color = if monitor || stand {
                [245, 248, 255, 255]
            } else {
                [38, 132, 220, 255]
            };
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).map_err(|error| error.to_string())
}
