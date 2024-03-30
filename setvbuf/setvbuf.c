#include <stdio.h>
#include <stdlib.h>

/**
 * This script is used to set the buffering mode of the standard output stream (stdout) to line buffered.
 * Line buffering means that the output will be flushed whenever a newline character ('\n') is encountered.
 * 
 * The purpose of setting the buffering mode to line buffered is to ensure that the output is displayed immediately
 * when writing to stdout, rather than waiting for the buffer to be full or manually flushing it.
 * 
 * This script is designed to be called when the library is loaded, using the __attribute__((constructor)) attribute.
 * It sets the buffering mode of stdout to line buffered by calling the setvbuf() function with the appropriate parameters.
 * 
 * Note: The buffer will be allocated by the library and should be BUFSIZ in size.
 * 
 * It's important to note that not all libraries support controlling the buffering mode of stdout. This library can be used
 * to modify the behavior of stdout buffering in tools that don't provide a built-in mechanism for it.
 */
void __attribute__((constructor)) initLibrary(void) {
    setvbuf(stdout, NULL, _IOLBF, BUFSIZ);
}