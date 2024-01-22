# OpenTitan CW310 Ubuntu Container Development Environment

Loading a bitstream:
```
~# cd opentitan
~/opentitan# git diff
diff --git a/sw/host/opentitanlib/src/transport/chip_whisperer/mod.rs b/sw/host/opentitanlib/src/transport/chip_whis
perer/mod.rs
index 5cc8fb33ea..6ba9cf3745 100644
--- a/sw/host/opentitanlib/src/transport/chip_whisperer/mod.rs
+++ b/sw/host/opentitanlib/src/transport/chip_whisperer/mod.rs
@@ -45,8 +45,9 @@ impl<B: Board> ChipWhisperer<B> {
         usb_vid: Option<u16>,
         usb_pid: Option<u16>,
         usb_serial: Option<&str>,
-        uart_override: &[&str],
+        _uart_override: &[&str],
     ) -> anyhow::Result<Self> {
+        let uart_override = ["/dev/boards/cw310-0/uart", "/dev/boards/cw310-0/control"];
         let board = ChipWhisperer {
             device: Rc::new(RefCell::new(usb::Backend::new(
                 usb_vid, usb_pid, usb_serial,
~/opentitan# ./bazelisk.sh test --test_output=streamed //sw/device/tests:uart_smoketest_fpga_cw310_test_rom
```

Loading Tock:
```
~/opentitan# ./bazelisk.sh run //sw/host/opentitantool -- --interface=cw310 bootstrap  /root/tock/target/riscv32imc-unknown-none-elf/release/earlgrey-cw310.bin
```

Connecting to the serial console:
```
~/opentitan# ./bazelisk.sh run //sw/host/opentitantool -- --interface=cw310 console
```
