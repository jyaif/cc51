#ifndef __CC51_CTYPE_H
#define __CC51_CTYPE_H
#define isdigit(c) ((unsigned char)((c) - '0') < 10)
#define isupper(c) ((unsigned char)((c) - 'A') < 26)
#define islower(c) ((unsigned char)((c) - 'a') < 26)
#define isalpha(c) ((unsigned char)(((c) | 0x20) - 'a') < 26)
#define isalnum(c) (isalpha(c) || isdigit(c))
#define isxdigit(c) (isdigit(c) || (unsigned char)(((c) | 0x20) - 'a') < 6)
#define isspace(c) ((c) == ' ' || (unsigned char)((c) - '\t') < 5)
#define isprint(c) ((unsigned char)((c) - ' ') < 95)
#define iscntrl(c) ((unsigned char)(c) < 32 || (c) == 127)
#define ispunct(c) (isprint(c) && !isalnum(c) && (c) != ' ')
#define isgraph(c) ((unsigned char)((c) - '!') < 94)
#define toupper(c) (islower(c) ? (c) - 32 : (c))
#define tolower(c) (isupper(c) ? (c) + 32 : (c))
#endif
