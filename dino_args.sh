M=~/Code/minitel-native
D=$M/examples/dino_game
MODEL=${MODEL:-nfz330}
DINO_ARGS="-I$M/lib/board/$MODEL/include -I$M/lib/keyboard/include -I$D/out/minitel_keyboard/include -I$M/lib/timer/include -I$M/lib/video/include -I$D/out/generated -DBOARD_$(echo $MODEL | tr a-z A-Z)
 $D/display.c $D/dynamic_area.c $D/enemies.c $D/floor_area.c $D/main.c $D/random.c $D/score.c $D/splash.c $D/static_area.c
 $M/lib/board/$MODEL/controls.c $M/lib/board/$MODEL/read_keyboard.c $M/lib/keyboard/key_is_pressed.c $M/lib/keyboard/key_to_name.c $M/lib/timer/timer.c"
