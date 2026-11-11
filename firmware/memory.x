/* APM32F103CB: 128K flash in 1K pages, 20K RAM. The stm32duino bootloader owns the first 8K.
   Above the image come the logo name (0x0801F400, src/name.rs) and the crash log
   (0x0801F800, src/diag.rs); the last page (0x0801FC00) is unused. The image must end below
   the logo name. */
MEMORY
{
  FLASH : ORIGIN = 0x08002000, LENGTH = 117K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}
