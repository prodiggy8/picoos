use embassy_rp::peripherals::{DMA_CH2, DMA_CH3, PIN_8, PIN_9, UART1};
use embassy_rp::uart::{Async, Config, Uart, InterruptHandler, TxPin, RxPin};
use embassy_rp::bind_interrupts;
use embassy_rp::Peri;



bind_interrupts!(pub struct Irqs {
    UART1_IRQ => InterruptHandler<UART1>;
});

pub struct Dvi<'a> {
    uart: Uart<'a, Async>,
}

impl<'a> Dvi<'a> {
    pub fn new(
        uart: Peri<'a, UART1>,
        tx_pin: Peri<'static, PIN_8>,
        rx_pin: Peri<'static, PIN_9>,
        tx_dma: Peri<'static, DMA_CH2>,
        rx_dma: Peri<'static, DMA_CH3>,
    ) -> Self {
        let mut config = Config::default();
        config.baudrate = 9600;

        let uart = Uart::new(uart, tx_pin, rx_pin, Irqs, tx_dma, rx_dma, config);
        Self { uart }
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), embassy_rp::uart::Error> {
        self.uart.write(data).await
    }
}

    