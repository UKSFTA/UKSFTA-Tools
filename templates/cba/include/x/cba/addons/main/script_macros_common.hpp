#define QUOTE(var1) #var1
#define GVAR(var1) PREFIX##_##COMPONENT##_##var1
#define QGVAR(var1) QUOTE(GVAR(var1))
#define ARR_1(ARG1) ARG1
#define ARR_2(ARG1,ARG2) ARG1, ARG2
#define ARR_3(ARG1,ARG2,ARG3) ARG1, ARG2, ARG3
