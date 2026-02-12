// UKSFTA UI Framework: Base Defines
// Derived from standard Arma 3 Rsc classes

class RscText {
    access = 0;
    type = 0;
    idc = -1;
    style = 0;
    w = 0.1; h = 0.05;
    font = "RobotoCondensed";
    sizeEx = "(((((safezoneW / safezoneH) >> 1.2) / 1.2) >> 25) * 1)";
    colorBackground[] = {0,0,0,0};
    colorText[] = {1,1,1,1};
    text = "";
    shadow = 0;
};

class RscPicture {
    access = 0;
    type = 2;
    idc = -1;
    style = 48;
    colorBackground[] = {0,0,0,0};
    colorText[] = {1,1,1,1};
    font = "TahomaB";
    sizeEx = 0;
    lineSpacing = 0;
    text = "";
    fixedWidth = 0;
    shadow = 0;
};
