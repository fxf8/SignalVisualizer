#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::delay_ms;
use avr_device::interrupt;
use panic_halt as _;

use core::cell::Cell;

static MILLISECOND_COUNTER: interrupt::Mutex<Cell<u32>> = interrupt::Mutex::new(Cell::new(0));

#[interrupt(atmega328p)]
fn TIMER0_OVF() {
    // Executes on 8 bit timer overflow
    interrupt::free(|cs| {
        let counter = MILLISECOND_COUNTER.borrow(cs);
        let next_val = counter.get().wrapping_add(1);
        counter.set(next_val);
    });
}

#[arduino_hal::entry]
fn main() -> ! {
    // Device peripherals
    let db = arduino_hal::Peripherals::take().unwrap();

    let pins = arduino_hal::pins!(db);

    let mut serial = arduino_hal::default_serial!(db, pins, 57600);
    let mut adc = arduino_hal::Adc::new(db.ADC, Default::default());

    // Used for monotonic clock
    let timer0 = db.TC0;

    // Used for msgeq7 ckin
    let timer2 = db.TC2;

    // Monotonic Clock Configuration
    // Timer 0 (Used for asynchrony)
    // Make the 8 bit timer only count every 64 clock cycles
    timer0.tccr0b().write(|w| w.cs0().prescale_64());
    // Enable overflow interrupt
    timer0.timsk0().write(|w| w.toie0().set_bit());

    // Timer 2 (Used for MSGEQ7 Clock Generation)
    timer2.tccr2a().write(|w| w.com2b().toggle().wgm2().ctc());
    timer2.tccr2b().write(|w| w.cs2().prescale_1());
    timer2.ocr2a().write(|w| unsafe { w.bits(47) });

    let _msgeq7_clock = pins.d3.into_output(); // Already driven by TC2 hardware
    let mut strobe = pins.d2.into_output();
    let mut reset = pins.d4.into_output();

    let mut measure = pins.a0.into_analog_input(&mut adc);

    let mut audio_band_amplitudes: [u16; 7] = [0; 7];

    loop {
        // ufmt::uwriteln!(&mut serial, "Hello from arduino ({})", count).unwrap();
        let mut value = measure.analog_read(&mut adc);

        ufmt::uwriteln!(&mut serial, "Read value ({}) from a0", value).unwrap();

        delay_ms(100);
    }
}
