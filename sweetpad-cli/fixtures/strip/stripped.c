#include <stdio.h>
extern char **environ;
int main(void){ puts("a"); return environ != 0; }
