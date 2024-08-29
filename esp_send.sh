## test bed 103
set HWID 6867254EED84
# 2(Emerg) pulse 500 - 5(Assist) pulse 1000 - 6(Cancel) pulse 500-  0(Dry1) on/off - 1(Dry2) on/off - 3(check-in) pulse 600
set ESP_LIGHTS 7E0002{$HWID}020205  7E0002{$HWID}050205  7E0002{$HWID}060205  7E0002{$HWID}000100 7E0002{$HWID}000000 7E0002{$HWID}010100 7E0002{$HWID}010000 7E0002{$HWID}030205 
set ESP_LIGHTS_TIMES 10 60 60 60 60 60 60 7200
## test call lights
set HWID 7CDFA1DEE298
#pulse
set ESP_LIGHTS 7E0002{$HWID}000205 7E0002{$HWID}010205 7E0002{$HWID}020205 7E0002{$HWID}030205 7E0002{$HWID}040205 7E0002{$HWID}050205 7E0002{$HWID}060205 7E0002{$HWID}070205
set ESP_LIGHTS_TIMES 5 5 5 5 5 5 5 5 1200


#command
cargo run --release -- -vvv test -p /dev/cu.SLAB_USBtoUART10 --esp-test --send $ESP_LIGHTS --send-time 20