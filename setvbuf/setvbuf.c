#include <stdio.h>
#include <stdlib.h>

/**
 * This program sets the buffering mode of stdout to line buffered.
 * 
 * It's a hacky way to make sure that the output of the program is
 * line buffered, even if the program is run in a non-interactive
 * environment.
 */
void __attribute__((constructor)) initLibrary(void) {
    setvbuf(stdout, NULL, _IOLBF, BUFSIZ);
}
