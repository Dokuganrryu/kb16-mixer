/* APM32F103CB: 128K flash, 20K RAM. The stm32duino bootloader owns the first 8K. */
MEMORY
{
  FLASH : ORIGIN = 0x08002000, LENGTH = 120K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}
