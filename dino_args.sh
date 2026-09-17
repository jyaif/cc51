M=~/Code/minitel-native
D=$M/examples/dino_game
MODEL=${MODEL:-nfz330}
if [ "$MODEL" = nfz330 ]; then
  KBINC=$D/out/minitel_keyboard/include
else
  KBINC=/private/tmp/claude-501/-Users-jf-Code-cc51/b138382e-02e1-4af0-90d6-2a9bb7f068f6/scratchpad/kb_$MODEL
fi
DINO_SRCS="$D/display.c $D/dynamic_area.c $D/enemies.c $D/floor_area.c $D/main.c $D/random.c $D/score.c $D/splash.c $D/static_area.c
 $M/lib/board/$MODEL/controls.c $M/lib/board/$MODEL/read_keyboard.c $M/lib/keyboard/key_is_pressed.c $M/lib/keyboard/key_to_name.c $M/lib/timer/timer.c"
DINO_INC="-I$M/lib/board/$MODEL/include -I$M/lib/keyboard/include -I$KBINC -I$M/lib/timer/include -I$M/lib/video/include -I$D/out/generated -DBOARD_$(echo $MODEL | tr a-z A-Z)"
DINO_ARGS="$DINO_INC $DINO_SRCS"
