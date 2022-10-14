use std::{io::{self, Write, Read, StdoutLock}, sync::{mpsc, Arc, Mutex}, thread, fs::File, time::Duration, mem};
use crossterm::{execute, terminal::{enable_raw_mode, disable_raw_mode}, cursor};
use num_bigint::{BigUint, ToBigUint};
use serialport::SerialPort;
use stopwatch::Stopwatch;
use winit::{event::Event, event_loop::{ControlFlow, EventLoop}};
use trayicon::{MenuBuilder, TrayIconBuilder, TrayIcon};
use termcolor::*;
use windows::{Win32::{Graphics::Gdi::*, Foundation::{BOOL, HWND}, UI::WindowsAndMessaging::*}, core::PCSTR};

/* #region MACROS */

static mut COLOR: ColorSpec = unsafe {const_zero::const_zero!(ColorSpec)};
macro_rules! color { // !NOT! thread safe. This is on purpose — color!() should only be used when a lock on io::stdout() is acquired.
    ($type:ident)       => {unsafe {COLOR.set_fg(Some(Color::$type)).set_intense(false)}};
    ($type:ident, true) => {unsafe {COLOR.set_fg(Some(Color::$type)).set_intense(true)}};
}
macro_rules! clr_print {
    ($stdout:expr, $color:ident, $($arg:tt)*) => {
        ($stdout).set_color(color!($color, true)).unwrap();
        print!($($arg)*)
    };
    ($stdout:expr, ($color:ident, true), $($arg:tt)*) => {
        ($stdout).set_color(color!($color, true)).unwrap();
        print!($($arg)*)
    };
}
macro_rules! clr_write {
    ($stdout:expr, $color:ident, $($arg:tt)*) => {
        ($stdout).set_color(color!($color, true)).unwrap();
        write!($($arg)*).unwrap();
    };
    ($stdout:expr, ($color:ident, true), $($arg:tt)*) => {
        ($stdout).set_color(color!($color, true)).unwrap();
        write!($($arg)*).unwrap();
    };
}

/* #endregion */

/* #region ENUMS */

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
#[repr(u8)]
enum Orientation {
    Landscape = DMDO_DEFAULT as u8,
    Portrait  = DMDO_90 as u8,
    LandscapeFlipped = DMDO_180 as u8,
    PortraitFlipped = DMDO_270 as u8
}
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
enum Events {
    //CudaFactorial,
    Factorial,
    SerialEnum, SerialTestComms,
    SerialQueryStatus, SerialIMURecalibrate,
    SerialAutoRotateMonitor,
    SerialRotateMonitor(Orientation),
    SerialPortChanged(usize),

    HideConsole, RefreshMenu,
    Exit//, None
}

/* #endregion */

/* #region CONSTANTS */

const FACTORIAL_THREAD_COUNT: u64 = 32;
const SERIAL_DEFAULT_NAME: &'static str = "COM4";
const SERIAL_BAUD_RATE: u32 = 9600;
const SERIAL_ACK_TIMEOUT: i64 = 10000;
const AUTOROTATE_ID: u32 = 1;
const AUTOROTATE_THRESHOLD_DEG: u8 = 65;

const SYN: u8 = 0x16;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const ENQ: u8 = 0x05;
const DC1: u8 = 0x11;
const DC2: u8 = 0x12;
const DC3: u8 = 0x13;
const DC4: u8 = 0x14;

/* #endregion */

fn main() {
    /* #region STARTUP */
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    unsafe {COLOR = ColorSpec::new();}

    /*clr_print!(stdout, (Magenta, true), "<——————————————————————————————————————————————————————————————————————————————————————>");
    clr_print!(stdout, (Cyan, true), r"
    ______                      __  __    _                 __                "); clr_print!(stdout, (Magenta, true), "███ ██ ██"); clr_print!(stdout, (Cyan, true), r"
   / ____/   _____  _______  __/ /_/ /_  (_)___  ____ _____/ /___  ___  _____ "); clr_print!(stdout, (Magenta, true), " █  █ █ █"); clr_print!(stdout, (Cyan, true), r"
  / __/ | | / / _ \/ ___/ / / / __/ __ \/ / __ \/ __ `/ __  / __ \/ _ \/ ___/          
 / /___ | |/ /  __/ /  / /_/ / /_/ / / / / / / / /_/ / /_/ / /_/ /  __/ /              
/_____/ |___/\___/_/   \__, /\__/_/ /_/_/_/ /_/\__, /\__,_/\____/\___/_/               
                      /____/                  /____/                                   
");// */
    clr_print!(stdout, (Magenta, true), "<——————————————————————————————————————————————————————————————————————————————————————>\n");
    io::stdout().flush().unwrap();

    let event_loop = EventLoop::with_user_event();
    let icon = include_bytes!("icon.ico");

    let serial_port = Arc::new(Mutex::new(None));
    let mut current_ori;
    /* #endregion */

    /* #region TASKBAR MENU SETUP */
    macro_rules! menu {
        () => {MenuBuilder::new()
            /*.with(MenuItem::Item {
                id: Events::None,
                name: "Everythingdoer™".to_string(),
                disabled: true,
                icon: Some(Icon::from_buffer(icon, None, None).unwrap())
            })*/
            
            .separator()

            .item("Refresh menu", Events::RefreshMenu)
            .checkable("Hide console", false, Events::HideConsole)
            
            .separator()

            .submenu("Serial (Arduino)", {
                let mut ret = MenuBuilder::new()
                    .item("Print serial ports", Events::SerialEnum)
                    .item("Test comms with selected port", Events::SerialTestComms)
                    .item("Query selected port", Events::SerialQueryStatus)
                    .item("Recalibrate selected port's IMU", Events::SerialIMURecalibrate)
                    .submenu("Monitor", unsafe {
                        let mut d = DISPLAY_DEVICEA::default();
                        d.cb = mem::size_of::<DISPLAY_DEVICEA>() as u32;
                        let mut dm = DEVMODEA::default();
                        
                        EnumDisplayDevicesA(PCSTR::null(), AUTOROTATE_ID, &mut d, 0);
                        EnumDisplaySettingsA(PCSTR::from_raw(mem::transmute(&d.DeviceName)), ENUM_CURRENT_SETTINGS, &mut dm);
                        current_ori = dm.Anonymous1.Anonymous2.dmDisplayOrientation;
                        
                        MenuBuilder::new()
                            .checkable("Landscape",           current_ori == Orientation::Landscape as u32,        Events::SerialRotateMonitor(Orientation::Landscape))
                            .checkable("Landscape (flipped)", current_ori == Orientation::LandscapeFlipped as u32, Events::SerialRotateMonitor(Orientation::LandscapeFlipped))
                            .checkable("Portrait",            current_ori == Orientation::Portrait as u32,         Events::SerialRotateMonitor(Orientation::Portrait))
                            .checkable("Portrait (flipped)",  current_ori == Orientation::PortraitFlipped as u32,  Events::SerialRotateMonitor(Orientation::PortraitFlipped))
                            .checkable("Auto-rotate", false, Events::SerialAutoRotateMonitor)
                    }).separator();
                
                let mut default_not_found = true;
                for (i, port) in serialport::available_ports().unwrap().iter().enumerate() {
                    let mut check = false;
                    if port.port_name == SERIAL_DEFAULT_NAME {
                        check  = true;
                        default_not_found = false;
                        match serialport::new(SERIAL_DEFAULT_NAME, SERIAL_BAUD_RATE).open() {
                            Ok(mut sp) => {
                                let mut stdoutl = io::stdout().lock();
                                clr_write!(stdout, (Cyan, true), stdoutl, "Opened serial port \"{SERIAL_DEFAULT_NAME}\".\n");
                                stdoutl.flush().unwrap();
                                sp.set_timeout(Duration::from_millis(1000)).unwrap();
                                sp.set_data_bits(serialport::DataBits::Eight).unwrap();
                                sp.set_flow_control(serialport::FlowControl::None).unwrap();
                                sp.set_parity(serialport::Parity::None).unwrap();
                                sp.set_stop_bits(serialport::StopBits::One).unwrap();
                                sp.write_data_terminal_ready(true).unwrap();
                                *serial_port.lock().unwrap() = Some(sp);
                            }
                            Err(e) => {
                                winconsole::window::show(true);
                                let mut stdoutl = io::stdout().lock();
                                clr_write!(stdout, (Red, true), stdoutl, "Couldn't open serial port \"{SERIAL_DEFAULT_NAME}\": ");
                                clr_write!(stdout, Red, stdoutl, "{}\n", e.to_string());
                                stdoutl.flush().unwrap();
                            }
                        }
                    }
                    ret = ret.checkable(&port.port_name, check, Events::SerialPortChanged(i));
                }

                if default_not_found {
                    winconsole::window::show(true);
                    let mut stdoutl = io::stdout().lock();
                    clr_write!(stdout, (Red, true), stdoutl, "Serial port \"{SERIAL_DEFAULT_NAME}\" not found.\n");
                    stdoutl.flush().unwrap();
                }

                ret
            })
            .item("Factorial calc", Events::Factorial)

            .separator()

            .item("Exit", Events::Exit)
        }
    }

    let tray_icon = Arc::new(Mutex::new(TrayIconBuilder::new()
        .sender_winit(unsafe {std::mem::transmute(event_loop.create_proxy())}) //it is literally the same struct shut up compiler
        .icon_from_buffer(icon)
        .tooltip("Everythingdoer™")
        .on_click(Events::HideConsole)
        .on_double_click(Events::RefreshMenu)
        .menu(menu!()).build().unwrap()));
    /* #endregion */

    /* #region SERIAL LISTENER THREAD */
    let serial_port_t = Arc::clone(&serial_port);
    let tray_icon_t = Arc::clone(&tray_icon);
    let mut stdout_t = StandardStream::stdout(ColorChoice::Always);
    thread::spawn(move || loop {
        {
            let mut tray_lock = tray_icon_t.lock().unwrap();
            if let Some(v) = tray_lock.get_menu_item_checkable(Events::SerialAutoRotateMonitor) {
                if v {
                    if let Some(ref mut port) = *serial_port_t.lock().unwrap() {
                        /*{
                            let mut stdoutl = io::stdout().lock();
                            writeln!(stdoutl, "ok");
                            stdoutl.flush().unwrap();
                        }*/
                        match port.bytes_to_read() {
                            Ok(n) => if n > 0 {
                                let mut buffer = [0u8];
                                port.read_exact(&mut buffer).unwrap();
        
                                let mut stdoutl = io::stdout().lock();
                                let (ori, str) = match buffer[0] {
                                    DC1 => (Orientation::Landscape,        "DC1"),
                                    DC2 => (Orientation::Portrait,         "DC2"),
                                    DC3 => (Orientation::LandscapeFlipped, "DC3"),
                                    DC4 => (Orientation::PortraitFlipped,  "DC4"),
                                    _ => (Orientation::Landscape, "\0")
                                };
                                if str != "\0" {
                                    clr_write!(stdout_t, (Cyan, true), stdoutl, "Received ");
                                    clr_write!(stdout_t, (Magenta, true), stdoutl, "{str}");
                                    clr_write!(stdout_t, (Cyan, true), stdoutl, " (");
                                    clr_write!(stdout_t, (Magenta, true), stdoutl, "{ori:?}");
                                    clr_write!(stdout_t, (Cyan, true), stdoutl, "), sending ");
                                    clr_write!(stdout_t, (Magenta, true), stdoutl, "ACK");
                                    clr_write!(stdout_t, (Cyan, true), stdoutl, "... ");
                                
                                    stdoutl.flush().unwrap();
                                
                                    if let Err(e) = port.write(&[ACK]) {
                                        clr_write!(stdout_t, (Red, true), stdoutl, "ERRT: Couldn't write - ");
                                        clr_write!(stdout_t, Red, stdoutl, "{}", e.to_string());
                                    } else {
                                        if let Err(e) = port.flush() {
                                            clr_write!(stdout_t, (Red, true), stdoutl, "ERRT: Couldn't flush - ");
                                            clr_write!(stdout_t, Red, stdoutl, "{}", e.to_string());
                                        } else {
                                            clr_write!(stdout_t, Green, stdoutl, "Success");
                                            if ori as u32 != current_ori {
                                                clr_write!(stdout_t, (Cyan, true), stdoutl, ", rotating monitor... ");
                                                rotate_monitor(AUTOROTATE_ID, ori, &mut tray_lock, &mut current_ori, false);
                                            } else {
                                                clr_write!(stdout_t, (Cyan, true), stdoutl, ", monitor already in requested orientation.\n");
                                            }
                                        }
                                    }
                                }
                                //else {clr_write!(stdout_t, (Cyan, true), stdoutl, "Received {}.\n", buffer[0]);}
                                stdoutl.flush().unwrap();
                            } /*else {
                                let mut stdoutl = io::stdout().lock();
                                writeln!(stdoutl, "{n}");
                                stdoutl.flush().unwrap();
                            }*/
                            Err(e) => {
                                let mut stdoutl = io::stdout().lock();
                                clr_write!(stdout_t, (Red, true), stdoutl, "ERRT: Couldn't get bytes to read - ");
                                clr_write!(stdout_t, Red, stdoutl, "{}\n", e.to_string());
                                stdoutl.flush().unwrap();
                                break;
                            }
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    });
    /* #endregion */
    
    /* #region NEW WINDOW HANDLER */
    /*unsafe {
        extern "system" fn hookproc() {0}
        let hhook = SetWindowsHookExA(WINDOWS_HOOK_ID(10), Some(hookproc), 0, );
    }*/
    /* #endregion */

    /* #region STARTUP EVENTS THREAD */
    let proxy_t = event_loop.create_proxy();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        if proxy_t.send_event(Events::SerialAutoRotateMonitor).is_ok() {
            thread::sleep(Duration::from_secs(1));
            _=proxy_t.send_event(Events::HideConsole);
        }
    });
    /* #endregion */
    
    //MAIN LOOP
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::UserEvent(e) = event {
            {
                let mut stdoutl = io::stdout().lock();
                clr_write!(stdout, (Cyan, true), stdoutl, "Events");
                clr_write!(stdout, (Magenta, true), stdoutl, "::");
                clr_write!(stdout, (Cyan, true), stdoutl, "{e:?}\n");
                stdoutl.flush().unwrap();
            }

            match e {
                Events::Exit => {
                    let mut stdoutl = io::stdout().lock();
                    let mut stdout = StandardStream::stdout(ColorChoice::Always);
                    
                    if let Some(ref mut port) = *serial_port.lock().unwrap() {
                        let mut tray_lock = tray_icon.lock().unwrap();
                        if let Some(oldv) = tray_lock.get_menu_item_checkable(Events::SerialAutoRotateMonitor) {
                            console_to_fg(&mut tray_lock);
                            serial_send(port, &[DC2], "DC2", |p: &mut Box<dyn SerialPort>, stdoutl: &mut StdoutLock| {
                                let mut buffer = [0u8];
                                p.read_exact(&mut buffer).unwrap();
            
                                if buffer[0] == ACK {
                                    clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                    clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                    clr_write!(stdout, (Magenta, true), stdoutl, "ACK");
                                    clr_write!(stdout, (Cyan, true), stdoutl, " received.\n");
                                    _=tray_lock.set_menu_item_checkable(Events::SerialAutoRotateMonitor, !oldv);
                                } else {
                                    clr_write!(stdout, (Red, true), stdoutl, "ERR: Received non-ACK code \"{:#04x}\".\n", buffer[0]);
                                }
                            });
                        }
                    }
                    clr_write!(stdout, (Magenta, true), stdoutl, "<——————————————————————————————————————————————————————————————————————————————————————>");
                    stdoutl.flush().unwrap();
                    _=stdout.reset();
            
                    console_to_fg(&mut tray_icon.lock().unwrap());
                    *control_flow = ControlFlow::Exit;
                }
                Events::HideConsole => {
                    let mut tray_lock = tray_icon.lock().unwrap();
                    if let Some(oldv) = tray_lock.get_menu_item_checkable(Events::HideConsole) {
                        _=tray_lock.set_menu_item_checkable(Events::HideConsole, !oldv);

                        if oldv {winconsole::window::show(true);}
                        else    {winconsole::window::hide();}
                    }
                }
                Events::RefreshMenu => {
                    let mut tray_lock = tray_icon.lock().unwrap();
                    if let Err(e) = tray_lock.set_menu(&menu!()) {
                        console_to_fg(&mut tray_lock);
                        let mut stdoutl = io::stdout().lock();
                        clr_write!(stdout, (Red, true), stdoutl, "Couldn't refresh: ");
                        clr_write!(stdout, Red, stdoutl, "{}\n", e.to_string());
                        stdoutl.flush().unwrap();
                    }
                }

                Events::SerialRotateMonitor(ori) => rotate_monitor(AUTOROTATE_ID, ori, &mut tray_icon.lock().unwrap(), &mut current_ori, true),
                Events::SerialAutoRotateMonitor => {
                    if let Some(ref mut port) = *serial_port.lock().unwrap() {
                        let mut tray_lock = tray_icon.lock().unwrap();
                        if let Some(oldv) = tray_lock.get_menu_item_checkable(Events::SerialAutoRotateMonitor) {
                            console_to_fg(&mut tray_lock);
                            serial_send(port, &[if oldv {DC2} else {DC1}], if oldv {"DC2"} else {"DC1"}, |p: &mut Box<dyn SerialPort>, stdoutl: &mut StdoutLock| {
                                let mut buffer = [0u8];
                                p.read_exact(&mut buffer).unwrap();
            
                                match buffer[0] {
                                    ACK => {
                                        clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                        clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                        clr_write!(stdout, (Magenta, true), stdoutl, "ACK");
                                        clr_write!(stdout, (Cyan, true), stdoutl, " received.\n");
                                        _=tray_lock.set_menu_item_checkable(Events::SerialAutoRotateMonitor, !oldv);

                                        for &o in &[Orientation::Landscape, Orientation::Portrait, Orientation::LandscapeFlipped, Orientation::PortraitFlipped] {
                                            _=tray_lock.set_menu_item_checkable(Events::SerialRotateMonitor(o), (o as u32)==current_ori);
                                        }
                                    }
                                    NAK => {
                                        if oldv {clr_write!(stdout, (Red, true), stdoutl, "ERR: NAK received.\n");} else {
                                            clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                            clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                            clr_write!(stdout, (Magenta, true), stdoutl, "NAK");
                                            clr_write!(stdout, (Cyan, true), stdoutl, " received.\n");
                                            _=tray_lock.set_menu_item_checkable(Events::SerialAutoRotateMonitor, !oldv);
    
                                            for &o in &[Orientation::Landscape, Orientation::Portrait, Orientation::LandscapeFlipped, Orientation::PortraitFlipped] {
                                                _=tray_lock.set_menu_item_checkable(Events::SerialRotateMonitor(o), false);
                                            }
                                        }
                                    } 
                                    ENQ => {
                                        clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                        clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                        clr_write!(stdout, (Magenta, true), stdoutl, "ENQ");
                                        clr_write!(stdout, (Cyan, true), stdoutl, " received, sending current orientation... ");

                                        serial_send(p, &[current_ori as u8, AUTOROTATE_THRESHOLD_DEG], &format!("[{current_ori:?}, {AUTOROTATE_THRESHOLD_DEG}]"), |p: &mut Box<dyn SerialPort>, stdoutl: &mut StdoutLock| {
                                            p.read_exact(&mut buffer).unwrap();
                                            if buffer[0] == ACK {
                                                clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                                clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                                clr_write!(stdout, (Magenta, true), stdoutl, "ACK");
                                                clr_write!(stdout, (Cyan, true), stdoutl, " received.\n");
                                                _=tray_lock.set_menu_item_checkable(Events::SerialAutoRotateMonitor, !oldv);

                                                for &o in &[Orientation::Landscape, Orientation::Portrait, Orientation::LandscapeFlipped, Orientation::PortraitFlipped] {
                                                    _=tray_lock.set_menu_item_checkable(Events::SerialRotateMonitor(o), false);
                                                }
                                            } else {
                                                clr_write!(stdout, (Red, true), stdoutl, "ERR: Received non-ACK code \"{:#04x}\".\n", buffer[0]);
                                            }
                                        });
                                    }
                                    
                                    _ => {clr_write!(stdout, (Red, true), stdoutl, "ERR: Received non-ACK code \"{:#04x}\".\n", buffer[0]);}
                                }
                            });
                        }
                    }
                }
                Events::SerialEnum => {
                    console_to_fg(&mut tray_icon.lock().unwrap());
                    for port in serialport::available_ports().unwrap() {
                        let mut stdoutl = io::stdout().lock();
                        clr_write!(stdout, (Cyan, true), stdoutl, "{}", port.port_name);
                        clr_write!(stdout, (Magenta, true), stdoutl, ": ");
                        clr_write!(stdout, (Cyan, true), stdoutl, "{:?}\n", port.port_type);
                        stdoutl.flush().unwrap();
                    }
                }
                Events::SerialTestComms => {
                    if let Some(ref mut port) = *serial_port.lock().unwrap() {
                        console_to_fg(&mut tray_icon.lock().unwrap());
                        serial_send(port, &[SYN], "SYN", |p: &mut Box<dyn SerialPort>, stdoutl: &mut StdoutLock| {
                            let mut buffer = [0u8];
                            p.read_exact(&mut buffer).unwrap();
    
                            if buffer[0] == ACK {
                                clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                clr_write!(stdout, (Magenta, true), stdoutl, "ACK");
                                clr_write!(stdout, (Cyan, true), stdoutl, " received.\n");
                            } else {
                                clr_write!(stdout, (Red, true), stdoutl, "ERR: Received non-ACK code \"{:#04x}\".\n", buffer[0]);
                            }
                        });
                    }
                }
                Events::SerialQueryStatus => {
                    if let Some(ref mut port) = *serial_port.lock().unwrap() {
                        console_to_fg(&mut tray_icon.lock().unwrap());
                        serial_send(port, &[SYN], "ENQ", |p: &mut Box<dyn SerialPort>, stdoutl: &mut StdoutLock| {
                            let mut buffer = [0u8];
                            p.read_exact(&mut buffer).unwrap();
    
                            match buffer[0] {
                                ACK => {
                                    clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                    clr_write!(stdout, (Cyan,    true), stdoutl, ": received ");
                                    clr_write!(stdout, (Magenta, true), stdoutl, "ACK");
                                    clr_write!(stdout, (Cyan,    true), stdoutl, " - autorotation is ");
                                    clr_write!(stdout, Green, stdoutl, "running");
                                    clr_write!(stdout, (Cyan,    true), stdoutl, ".\n");
                                }
                                NAK => {
                                    clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                    clr_write!(stdout, (Cyan,    true), stdoutl, ": received ");
                                    clr_write!(stdout, (Magenta, true), stdoutl, "NAK");
                                    clr_write!(stdout, (Cyan,    true), stdoutl, " - autorotation is ");
                                    clr_write!(stdout, (Red,     true), stdoutl, "not running");
                                    clr_write!(stdout, (Cyan,    true), stdoutl, ".\n");
                                }
                                _ => {clr_write!(stdout, (Red, true), stdoutl, "ERR: Received non-ACK code \"{:#04x}\".\n", buffer[0]);}
                            }
                        });
                    }
                }
                Events::SerialIMURecalibrate => {
                    if let Some(ref mut port) = *serial_port.lock().unwrap() {
                        console_to_fg(&mut tray_icon.lock().unwrap());
                        serial_send(port, &[SYN], "DC3", |p: &mut Box<dyn SerialPort>, stdoutl: &mut StdoutLock| {
                            let mut buffer = [0u8];
                            p.read_exact(&mut buffer).unwrap();
    
                            if buffer[0] == ACK {
                                clr_write!(stdout, Green, stdoutl, "\nSuccess");
                                clr_write!(stdout, (Cyan, true), stdoutl, ": ");
                                clr_write!(stdout, (Magenta, true), stdoutl, "ACK");
                                clr_write!(stdout, (Cyan, true), stdoutl, " received.\n");
                            } else {
                                clr_write!(stdout, (Red, true), stdoutl, "ERR: Received non-ACK code \"{:#04x}\".\n", buffer[0]);
                            }
                        });
                    }
                }
                Events::SerialPortChanged(x) => {
                    let port = &serialport::available_ports().unwrap()[x].port_name;
                    match serialport::new(port, SERIAL_BAUD_RATE).open() {
                        Ok(mut sp) => {
                            sp.set_data_bits(serialport::DataBits::Eight).unwrap();
                            sp.set_flow_control(serialport::FlowControl::None).unwrap();
                            sp.set_parity(serialport::Parity::None).unwrap();
                            sp.set_stop_bits(serialport::StopBits::One).unwrap();
                            sp.set_timeout(Duration::from_millis(1000)).unwrap();
                            *serial_port.lock().unwrap() = Some(sp);
                        }
                        Err(e) => {
                            winconsole::window::show(true);
                            let mut stdoutl = io::stdout().lock();
                            clr_write!(stdout, (Red, true), stdoutl, "Couldn't open serial port \"{port}\": ");
                            clr_write!(stdout, Red, stdoutl, "{}\n", e.to_string());
                            stdoutl.flush().unwrap();
                        }
                    }

                    let mut i = 0;
                    let mut tray_lock = tray_icon.lock().unwrap();
                    while tray_lock.get_menu_item_checkable(Events::SerialPortChanged(i)).is_some() {
                        _=tray_lock.set_menu_item_checkable(Events::SerialPortChanged(i), i==x);
                        i+=1;
                    }
                }

                Events::Factorial => {
                    let hidden = console_to_fg(&mut tray_icon.lock().unwrap());
                    let n = {
                        let mut stdoutl = io::stdout().lock();
    
                        clr_write!(stdout, (Cyan, true), stdoutl, "Input number to calculate the factorial of: ");
                        stdoutl.flush().unwrap();
                        let n = input(|buffer| {
                            if let Ok(n) = buffer.trim().parse::<u64>() {Some(n)} else {None}
                        });
                        clr_write!(stdout, (Cyan, true), stdoutl, "Calculating...\n");
                        stdoutl.flush().unwrap();
                        n
                    };
                    let mut sw = Stopwatch::start_new();

                    //let thread_count = n/100;
                    let (tx, rx) = mpsc::sync_channel::<BigUint>(FACTORIAL_THREAD_COUNT as usize);
                    let result = Arc::new(Mutex::new(1u8.to_biguint().unwrap()));
                    let resultc = Arc::clone(&result);

                    thread::spawn(move || {
                        let mut resl = resultc.lock().unwrap();
                        let mut count = 0;
                        let mut _stdout = StandardStream::stdout(ColorChoice::Always);
                        while count <= FACTORIAL_THREAD_COUNT {
                            {
                                let mut stdoutl = io::stdout().lock();
                                execute!(stdoutl, cursor::MoveToColumn(0)).unwrap();
                                clr_write!(_stdout, (Cyan, true), stdoutl, "Threads finished: ");
                                clr_write!(_stdout, (Magenta, true), stdoutl, "{}", count);
                                clr_write!(_stdout, (Cyan, true), stdoutl, "/");
                                clr_write!(_stdout, (Magenta, true), stdoutl, "{}", FACTORIAL_THREAD_COUNT);
                                stdoutl.flush().unwrap();
                            }
                            if let Ok(rc) = rx.recv() {*resl *= rc;}
                            count+=1;
                        }
                    });

                    let ops = n/FACTORIAL_THREAD_COUNT;
                    for i in 0..FACTORIAL_THREAD_COUNT {
                        let tx_c = tx.clone();
                        thread::spawn(move || {
                            let _i = ops*i+1;
                            let mut local_result = 1u8.to_biguint().unwrap();
                            for j in _i.._i+ops {
                                local_result *= j;
                            }
                            tx_c.send(local_result).unwrap();
                        });
                    }

                    /*(0..thread_count).into_par_iter().for_each(move |i| {
                        let _i = i*100+1;
                        let mut local_result = 1u8.to_biguint().unwrap();
                        for j in _i.._i+100 {
                            local_result *= j;
                        }
                        tx.send(local_result).unwrap();
                    });*/
                    {_=result.lock().unwrap();}
                    let mut resl = Arc::try_unwrap(result).unwrap().into_inner().unwrap();
                    for i in n-n%FACTORIAL_THREAD_COUNT..n {
                        resl *= i+1;
                    }

                    let calc_time = sw.elapsed_ms();
                    {
                        let mut stdoutl = io::stdout().lock();
                        clr_write!(stdout, (Cyan, true), stdoutl, "\nCalculated, converting to decimal...");
                        stdoutl.flush().unwrap();
                    }
                    sw.restart();
                    
                    let reslv = resl.to_radix_be(10).into_iter().map(|c| {c+0x30}).collect::<Vec<u8>>();
                    let resl_sn1 = format!("{}.{}{}{}{}", reslv[0] as char, reslv[1] as char, reslv[2] as char, reslv[3] as char, reslv[4] as char);
                    let resl_sn2 = format!("{}", reslv.len()-1);

                    let mut buffer = [0u8];
                    {
                        let mut stdoutl = io::stdout().lock();
                        clr_write!(stdout, (Cyan, true),    stdoutl, "\nFactorial of ");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{n}");
                        clr_write!(stdout, (Cyan, true),    stdoutl, " = ");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{resl_sn1}");
                        clr_write!(stdout, (Cyan, true),    stdoutl, "e");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{resl_sn2}");
                        clr_write!(stdout, (Cyan, true),    stdoutl, ". Calculated in ");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{calc_time}ms");
                        clr_write!(stdout, (Cyan, true),    stdoutl, ", converted to decimal in ");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{}ms", sw.elapsed_ms());
                        clr_write!(stdout, (Cyan, true),    stdoutl, ". Save to file (space) or display (any)? ");
                        stdoutl.flush().unwrap();

    
                        enable_raw_mode().unwrap();
                        io::stdin().read(&mut buffer).unwrap();
                    }

                    if buffer[0] == b' ' {
                        {
                            let mut stdoutl = io::stdout().lock();
                            clr_write!(stdout, (Cyan, true), stdoutl, "\nWriting to file..."); stdoutl.flush().unwrap();
                            stdoutl.flush().unwrap();
                        }
                        let path = format!(r"C:\Users\Roman\Desktop\everythingdoer\src\factorial\{}.txt", n);
                        let mut f = File::create(&path).unwrap();

                        _=f.write_all(&reslv);
                        write!(f, "\nScientific notation: {resl_sn1}e{resl_sn2}\nCalculation time = {calc_time}ms");

                        _=open::that(path);
                    } else {
                        let mut stdoutl = io::stdout().lock();
                        clr_write!(stdout, (Cyan, true), stdoutl, "\nWriting to console...\n");
                        stdoutl.flush().unwrap();
                        _=io::stdout().write_all(&reslv);
                        stdoutl.flush().unwrap();
                        clr_write!(stdout, (Cyan, true), stdoutl, "\nScientific notation: ");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{resl_sn1}");
                        clr_write!(stdout, (Cyan, true),    stdoutl, "e");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{resl_sn2}");
                        clr_write!(stdout, (Cyan, true),    stdoutl, "\nCalculation time = ");
                        clr_write!(stdout, (Magenta, true), stdoutl, "{calc_time}ms");
                        stdoutl.flush().unwrap();
                    }
                    disable_raw_mode().unwrap();
                    println!("\nDone.");

                    if hidden {winconsole::window::hide();}
                }
                /*Events::CudaFactorial => {
                    let path = format!(r"{}\src\external\CudaFactorial\x64\Release\CudaFactorial.exe", env!("CARGO_MANIFEST_DIR")); //don't ever do this. ever.
                    _=std::process::Command::new(path).spawn().unwrap();
                }*/

                //_ => ()
            }
        }
    });
}


fn rotate_monitor(monitor_id: u32, ori: Orientation, tray_icon: &mut TrayIcon<Events>, current_ori: &mut u32, manual: bool) {
    unsafe {
        let mut d = DISPLAY_DEVICEA::default();
        d.cb = mem::size_of::<DISPLAY_DEVICEA>() as u32;
        let mut dm = DEVMODEA::default();
        
        if EnumDisplayDevicesA(
            PCSTR::null(), monitor_id, &mut d, 0
        ) != BOOL::from(false) {
            if EnumDisplaySettingsA(
                PCSTR::from_raw(mem::transmute(&d.DeviceName)),
                ENUM_CURRENT_SETTINGS, &mut dm
            ) != BOOL::from(false) {
                if (dm.Anonymous1.Anonymous2.dmDisplayOrientation + ori as u32)%2==1 {
                    let temp = dm.dmPelsHeight;
                    dm.dmPelsHeight = dm.dmPelsWidth;
                    dm.dmPelsWidth = temp;
                }

                dm.Anonymous1.Anonymous2.dmDisplayOrientation = ori as u32;

                let ret = ChangeDisplaySettingsExA(
                    PCSTR::from_raw(mem::transmute(&d.DeviceName)),
                    &dm, HWND::default(), CDS_UPDATEREGISTRY, mem::zeroed()
                );
                let mut stdout = StandardStream::stdout(ColorChoice::Always);
                if ret == DISP_CHANGE_SUCCESSFUL {
                    if manual {
                        for &o in &[Orientation::Landscape, Orientation::Portrait, Orientation::LandscapeFlipped, Orientation::PortraitFlipped] {
                            _=tray_icon.set_menu_item_checkable(Events::SerialRotateMonitor(o), o==ori);
                        }
                    }
                    let mut stdoutl = io::stdout().lock();
                    clr_write!(stdout, Green, stdoutl, "OK\n");
                    *current_ori = dm.Anonymous1.Anonymous2.dmDisplayOrientation;
                    stdoutl.flush().unwrap();
                } else {
                    let DISP_CHANGE(i) = ret;
                    let mut stdoutl = io::stdout().lock();
                    clr_write!(stdout, (Red, true), stdoutl, "Couldn't rotate screen: ");
                    clr_write!(stdout, Red, stdoutl, "DISP_CHANGE({})\n", i);
                    stdoutl.flush().unwrap();
                }
            }
        }
    }
}

fn serial_send<F, S: AsRef<str> + std::fmt::Display>(port: &mut Box<dyn SerialPort>, send: &[u8], send_str: S, mut on_recv: F)
where F: FnMut(&mut Box<dyn SerialPort>, &mut StdoutLock) {
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    let mut stdoutl = io::stdout().lock();
    clr_write!(stdout, (Cyan, true), stdoutl, "Sending ");
    clr_write!(stdout, (Magenta, true), stdoutl, "{send_str}");
    clr_write!(stdout, (Cyan, true), stdoutl, "...");

    stdoutl.flush().unwrap();

    if let Err(e) = port.write(send) {
        clr_write!(stdout, (Red, true), stdoutl, "ERR: Couldn't write - ");
        clr_write!(stdout, Red, stdoutl, "{}", e.to_string());
    } else {
        if let Err(e) = port.flush() {
            clr_write!(stdout, (Red, true), stdoutl, "ERR: Couldn't flush - ");
            clr_write!(stdout, Red, stdoutl, "{}", e.to_string());
        } else {
            clr_write!(stdout, Green, stdoutl, "\nSuccess");
            clr_write!(stdout, (Cyan, true), stdoutl, ", waiting for response...");
            stdoutl.flush().unwrap();
            
            let sw = Stopwatch::start_new();

            loop {
                match port.bytes_to_read() {
                    Ok(n) => if n > 0 {on_recv(port, &mut stdoutl); break;} else if sw.elapsed_ms() > SERIAL_ACK_TIMEOUT {
                        clr_write!(stdout, (Red, true), stdoutl, "ERR: Timeout.\n");
                        break;
                    }
                    Err(e) => {
                        clr_write!(stdout, (Red, true), stdoutl, "ERR: Couldn't get bytes to read - ");
                        clr_write!(stdout, Red, stdoutl, "{}\n", e.to_string());
                        break;
                    }
                }
            }
        }
    }
    stdoutl.flush().unwrap();
}

fn console_to_fg(tray_icon: &mut TrayIcon<Events>) -> bool {
    let mut hidden_before = true;
    if let Some(oldv) = tray_icon.get_menu_item_checkable(Events::HideConsole) {
        if !oldv {
            hidden_before = false;
            winconsole::window::hide();
        }
    }
    winconsole::window::show(true);
    hidden_before
}

fn input<T, F>(mut condition: F) -> T where F: FnMut(&str) -> Option<T> {
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    loop {
        let mut buffer = String::new();
        io::stdin().read_line(&mut buffer).unwrap();

        if let Some(v) = condition(&buffer) {break v}

        let mut stdoutl = io::stdout().lock();
        clr_write!(stdout, (Magenta, true), stdoutl, "Invalid value, try again: ");
        _=stdout.reset();
        stdoutl.flush().unwrap();
    }
}