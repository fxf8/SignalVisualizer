#![no_std]
#![no_main]

use arduino_hal::delay_ms;
use avr_device::interrupt;
use panic_halt as _;

use core::cell::Cell;

static MILLISECOND_COUNTER: interrupt::Mutex<Cell<u32>> = interrupt::Mutex::new(Cell::new(0));

#[arduino_hal::entry]
fn main() -> ! {
    let device_peripherals = arduino_hal::Peripherals::take().unwrap();

    let pins = arduino_hal::pins!(device_peripherals);

    let mut serial = arduino_hal::default_serial!(device_peripherals, pins, 57600);
    let mut analog_digital_converter = arduino_hal::Adc::new(device_peripherals.ADC, Default::default());

    let timer0 = device_peripherals.TC0;

    // timer0.tccr0b().write;

    let mut audio_band_amplitudes: [u16; 7] = [0; 7];

    let mut reset = pins.d2.into_output();
    let mut strobe = pins.d3.into_output();

    let mut measure = pins.a0.into_analog_input(&mut analog_digital_converter);

    loop {
        // ufmt::uwriteln!(&mut serial, "Hello from arduino ({})", count).unwrap();
        let mut value = measure.analog_read(&mut analog_digital_converter);

        ufmt::uwriteln!(&mut serial, "Read value ({}) from a0", value).unwrap();

        delay_ms(10);
    }
}
