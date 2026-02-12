#include "script_macros.hpp"

class CfgPatches {
    class uksfta_project_main {
        name = "UKSF Taskforce Alpha - Project Interface";
        units[] = {};
        weapons[] = {};
        requiredAddons[] = {"A3_Data_F", "UKSFTA_Mods_Main"};
        author = "UKSF Taskforce Alpha";
        version = "1.0.0";
    };
};

// UI Headers
#include "include\defines.hpp"

class RscTitles {
    // Custom HUDs go here
};

class CfgFunctions {
    class uksfta_project {
        tag = "uksfta_project";
        class main {
            file = "z\uksfta\addons\project\functions";
            class openLaptop {};
        };
    };
};
