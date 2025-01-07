## reset vars
set -eU ESP_LIGHTS ESP_LIGHTS_TIMES
## test bed 103
set HWID 6867254EED84
## test Bed 101 + CH 101
set HWID 6867254E3FF0
## test bed 108
set HWID A0764EAD1D30
# 6(CLEAR) pulse 500 -> 060205
# 0(Dry1/Tamper) Pulse Test -- 2(Red/1st Cord) pulse/clear - 5(Assist) pulse/clear - 3(AUX3/2nd Cord) on/off - 0(Dry1/Tamper) on/off - 1(Dry2) pulse/clear - 6(CLEAR) pulse 500
: test-esp-2;set -U ESP_LIGHTS 7E0002{$HWID}000100 7E0002{$HWID}000000 7E0002{$HWID}020205 7E0002{$HWID}060205 7E0002{$HWID}050205 7E0002{$HWID}060205 7E0002{$HWID}030100 7E0002{$HWID}030000 7E0002{$HWID}000100 7E0002{$HWID}000000 7E0002{$HWID}010205 7E0002{$HWID}060205 7E0002{$HWID}060205 $ESP_LIGHTS; set -U ESP_LIGHTS_TIMES 10 1 60 60 60 60 60 60 60 60 60 60 3000 $ESP_LIGHTS_TIMES
# 2(Red/1st Cord) pulse/clear - 5(Assist) pulse/clear - 3(AUX3/2nd Cord) pulse/clear - 0(Dry1) on/off - 1(Dry2) on/off - 6(CLEAR) pulse 500
: test-esp-1;set -U ESP_LIGHTS 7E0002{$HWID}020205 7E0002{$HWID}060205 7E0002{$HWID}050205 7E0002{$HWID}060205 7E0002{$HWID}030205 7E0002{$HWID}060205 7E0002{$HWID}000100 7E0002{$HWID}000000 7E0002{$HWID}010100 7E0002{$HWID}010000 7E0002{$HWID}060205 $ESP_LIGHTS; set -U ESP_LIGHTS_TIMES 10 60 60 60 60 60 60 60 60 60 3043 $ESP_LIGHTS_TIMES
## test call lights
set HWID 7CDFA1DEE298
# pulse all 8 outputs
: test-esp-lights;set ESP_LIGHTS 7E0002{$HWID}000205 7E0002{$HWID}010205 7E0002{$HWID}020205 7E0002{$HWID}030205 7E0002{$HWID}040205 7E0002{$HWID}050205 7E0002{$HWID}060205 7E0002{$HWID}070205 $ESP_LIGHTS;;set ESP_LIGHTS_TIMES 5 5 5 5 5 5 5 5 1200 $ESP_LIGHTS_TIMES

#command
cargo run --release -- -vvv test -p /dev/cu.SLAB_USBtoUART10 --esp-test --send $ESP_LIGHTS --send-time $ESP_LIGHTS_TIMES