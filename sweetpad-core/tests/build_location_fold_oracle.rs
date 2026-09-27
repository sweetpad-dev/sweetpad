//! Build-location folding oracle: `xcodebuild` folds `SYMROOT`, `OBJROOT`,
//! `DSTROOT`, `CONFIGURATION_BUILD_DIR` and a few more lexically once they
//! resolve (`/tmp/../tmp/x` is `/tmp/x`, no symlink resolved), reads a
//! relative one against the project's directory, and every setting built from
//! one follows. Any other path setting keeps its spelling.
//!
//! Each case below quotes `xcodebuild -showBuildSettings -json -project
//! Scratch.xcodeproj -scheme Scratch <args>` on Xcode 27.0 (27A266a), run on
//! a copy of the `_synthetic-xcconfigs` Scratch project with the arguments
//! shown. `<PROJECT_DIR>` stands for the project's directory, `<PARENT>` and
//! `<GRANDPARENT>` for the ones above it. The resolver gets the same
//! arguments against the fixture project and must print the same values.
//!
//! Left out: `SHARED_PRECOMPS_DIR` once `OBJROOT` moves. With a scheme,
//! `xcodebuild` keeps it under DerivedData's intermediates whatever `OBJROOT`
//! says, where the resolver derives it from `OBJROOT`.

mod common;

use std::path::{Path, PathBuf};

use sweetpad_core::build_context::{BuildContext, ResolveQuery};
use sweetpad_core::scratch::ScratchDir;

use common::CatalogCache;

fn scratch_project() -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR"))
        .join("fixtures/_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj")
}

/// One captured `xcodebuild` run: the `KEY=VALUE` settings it was given, the
/// `-xcconfig` body if any, and what it reported.
struct Case {
    name: &'static str,
    settings: &'static [&'static str],
    xcconfig: Option<&'static str>,
    expected: &'static [(&'static str, &'static str)],
}

const CASES: &[Case] = &[
    Case {
        name: "dot segments on the command line",
        settings: &[
            "SYMROOT=/tmp/../tmp/x/sym",
            "OBJROOT=/tmp/./a/../obj",
            "DSTROOT=/tmp/../tmp/dst",
            "MYPATH=/tmp/../tmp/y",
        ],
        xcconfig: None,
        expected: &[
            ("SYMROOT", "/tmp/x/sym"),
            ("OBJROOT", "/tmp/obj"),
            ("DSTROOT", "/tmp/dst"),
            ("BUILD_DIR", "/tmp/x/sym"),
            ("BUILD_ROOT", "/tmp/x/sym"),
            ("CONFIGURATION_BUILD_DIR", "/tmp/x/sym/Debug"),
            ("BUILT_PRODUCTS_DIR", "/tmp/x/sym/Debug"),
            ("TARGET_BUILD_DIR", "/tmp/x/sym/Debug"),
            ("TEMP_ROOT", "/tmp/obj"),
            ("PROJECT_TEMP_DIR", "/tmp/obj/Scratch.build"),
            ("CONFIGURATION_TEMP_DIR", "/tmp/obj/Scratch.build/Debug"),
            (
                "TARGET_TEMP_DIR",
                "/tmp/obj/Scratch.build/Debug/Scratch.build",
            ),
            ("TEMP_DIR", "/tmp/obj/Scratch.build/Debug/Scratch.build"),
            ("INSTALL_ROOT", "/tmp/dst"),
            ("INSTALL_DIR", "/tmp/dst/usr/local/bin"),
            ("INSTALL_PATH", "/usr/local/bin"),
            ("LOCROOT", "<PROJECT_DIR>"),
            ("LOCSYMROOT", "<PROJECT_DIR>"),
            ("SHARED_DERIVED_FILE_DIR", "/tmp/x/sym/Debug/DerivedSources"),
            (
                "DERIVED_FILE_DIR",
                "/tmp/obj/Scratch.build/Debug/Scratch.build/DerivedSources",
            ),
            (
                "OBJECT_FILE_DIR",
                "/tmp/obj/Scratch.build/Debug/Scratch.build/Objects",
            ),
            ("PROJECT_TEMP_ROOT", "/tmp/obj"),
            ("DWARF_DSYM_FOLDER_PATH", "/tmp/x/sym/Debug"),
            ("CODESIGNING_FOLDER_PATH", "/tmp/x/sym/Debug/Scratch"),
            ("MYPATH", "/tmp/../tmp/y"),
        ],
    },
    Case {
        name: "every location set to /tmp/../tmp/<KEY>",
        settings: &[
            "SYMROOT=/tmp/../tmp/SYMROOT",
            "OBJROOT=/tmp/../tmp/OBJROOT",
            "DSTROOT=/tmp/../tmp/DSTROOT",
            "BUILD_DIR=/tmp/../tmp/BUILD_DIR",
            "BUILD_ROOT=/tmp/../tmp/BUILD_ROOT",
            "CONFIGURATION_BUILD_DIR=/tmp/../tmp/CONFIGURATION_BUILD_DIR",
            "BUILT_PRODUCTS_DIR=/tmp/../tmp/BUILT_PRODUCTS_DIR",
            "TARGET_BUILD_DIR=/tmp/../tmp/TARGET_BUILD_DIR",
            "TEMP_ROOT=/tmp/../tmp/TEMP_ROOT",
            "PROJECT_TEMP_DIR=/tmp/../tmp/PROJECT_TEMP_DIR",
            "CONFIGURATION_TEMP_DIR=/tmp/../tmp/CONFIGURATION_TEMP_DIR",
            "TARGET_TEMP_DIR=/tmp/../tmp/TARGET_TEMP_DIR",
            "TEMP_DIR=/tmp/../tmp/TEMP_DIR",
            "SHARED_PRECOMPS_DIR=/tmp/../tmp/SHARED_PRECOMPS_DIR",
            "INSTALL_ROOT=/tmp/../tmp/INSTALL_ROOT",
            "INSTALL_DIR=/tmp/../tmp/INSTALL_DIR",
            "INSTALL_PATH=/tmp/../tmp/INSTALL_PATH",
            "LOCROOT=/tmp/../tmp/LOCROOT",
            "LOCSYMROOT=/tmp/../tmp/LOCSYMROOT",
            "SHARED_DERIVED_FILE_DIR=/tmp/../tmp/SHARED_DERIVED_FILE_DIR",
            "DERIVED_FILE_DIR=/tmp/../tmp/DERIVED_FILE_DIR",
            "OBJECT_FILE_DIR=/tmp/../tmp/OBJECT_FILE_DIR",
            "PROJECT_TEMP_ROOT=/tmp/../tmp/PROJECT_TEMP_ROOT",
            "DWARF_DSYM_FOLDER_PATH=/tmp/../tmp/DWARF_DSYM_FOLDER_PATH",
            "CODESIGNING_FOLDER_PATH=/tmp/../tmp/CODESIGNING_FOLDER_PATH",
            "MODULE_CACHE_DIR=/tmp/../tmp/MODULE_CACHE_DIR",
        ],
        xcconfig: None,
        expected: &[
            ("SYMROOT", "/tmp/SYMROOT"),
            ("OBJROOT", "/tmp/OBJROOT"),
            ("DSTROOT", "/tmp/DSTROOT"),
            ("CONFIGURATION_BUILD_DIR", "/tmp/CONFIGURATION_BUILD_DIR"),
            ("BUILT_PRODUCTS_DIR", "/tmp/BUILT_PRODUCTS_DIR"),
            ("TARGET_BUILD_DIR", "/tmp/TARGET_BUILD_DIR"),
            ("CONFIGURATION_TEMP_DIR", "/tmp/CONFIGURATION_TEMP_DIR"),
            ("TARGET_TEMP_DIR", "/tmp/TARGET_TEMP_DIR"),
            ("TEMP_DIR", "/tmp/TEMP_DIR"),
            ("SHARED_PRECOMPS_DIR", "/tmp/SHARED_PRECOMPS_DIR"),
            ("INSTALL_DIR", "/tmp/INSTALL_DIR"),
            ("LOCROOT", "/tmp/LOCROOT"),
            ("LOCSYMROOT", "/tmp/LOCSYMROOT"),
            ("BUILD_DIR", "/tmp/../tmp/BUILD_DIR"),
            ("BUILD_ROOT", "/tmp/../tmp/BUILD_ROOT"),
            ("TEMP_ROOT", "/tmp/../tmp/TEMP_ROOT"),
            ("PROJECT_TEMP_DIR", "/tmp/../tmp/PROJECT_TEMP_DIR"),
            ("INSTALL_ROOT", "/tmp/../tmp/INSTALL_ROOT"),
            ("INSTALL_PATH", "/tmp/../tmp/INSTALL_PATH"),
            (
                "SHARED_DERIVED_FILE_DIR",
                "/tmp/../tmp/SHARED_DERIVED_FILE_DIR",
            ),
            ("DERIVED_FILE_DIR", "/tmp/../tmp/DERIVED_FILE_DIR"),
            ("OBJECT_FILE_DIR", "/tmp/../tmp/OBJECT_FILE_DIR"),
            ("PROJECT_TEMP_ROOT", "/tmp/../tmp/PROJECT_TEMP_ROOT"),
            (
                "DWARF_DSYM_FOLDER_PATH",
                "/tmp/../tmp/DWARF_DSYM_FOLDER_PATH",
            ),
            (
                "CODESIGNING_FOLDER_PATH",
                "/tmp/../tmp/CODESIGNING_FOLDER_PATH",
            ),
            ("MODULE_CACHE_DIR", "/tmp/../tmp/MODULE_CACHE_DIR"),
        ],
    },
    Case {
        name: "relative values on the command line",
        settings: &[
            "SYMROOT=a/../rel/SYMROOT",
            "OBJROOT=a/../rel/OBJROOT",
            "DSTROOT=a/../rel/DSTROOT",
            "CONFIGURATION_BUILD_DIR=a/../rel/CONFIGURATION_BUILD_DIR",
            "BUILT_PRODUCTS_DIR=a/../rel/BUILT_PRODUCTS_DIR",
            "TARGET_BUILD_DIR=a/../rel/TARGET_BUILD_DIR",
            "CONFIGURATION_TEMP_DIR=a/../rel/CONFIGURATION_TEMP_DIR",
            "TARGET_TEMP_DIR=a/../rel/TARGET_TEMP_DIR",
            "TEMP_DIR=a/../rel/TEMP_DIR",
            "SHARED_PRECOMPS_DIR=a/../rel/SHARED_PRECOMPS_DIR",
            "INSTALL_DIR=a/../rel/INSTALL_DIR",
            "LOCROOT=a/../rel/LOCROOT",
            "LOCSYMROOT=a/../rel/LOCSYMROOT",
            "BUILD_DIR=a/../rel/BUILD_DIR",
        ],
        xcconfig: None,
        expected: &[
            ("SYMROOT", "<PROJECT_DIR>/rel/SYMROOT"),
            ("OBJROOT", "<PROJECT_DIR>/rel/OBJROOT"),
            ("DSTROOT", "<PROJECT_DIR>/rel/DSTROOT"),
            ("BUILD_DIR", "a/../rel/BUILD_DIR"),
            ("BUILD_ROOT", "<PROJECT_DIR>/rel/SYMROOT"),
            (
                "CONFIGURATION_BUILD_DIR",
                "<PROJECT_DIR>/rel/CONFIGURATION_BUILD_DIR",
            ),
            ("BUILT_PRODUCTS_DIR", "<PROJECT_DIR>/rel/BUILT_PRODUCTS_DIR"),
            ("TARGET_BUILD_DIR", "rel/TARGET_BUILD_DIR"),
            ("TEMP_ROOT", "<PROJECT_DIR>/rel/OBJROOT"),
            (
                "PROJECT_TEMP_DIR",
                "<PROJECT_DIR>/rel/OBJROOT/Scratch.build",
            ),
            (
                "CONFIGURATION_TEMP_DIR",
                "<PROJECT_DIR>/rel/CONFIGURATION_TEMP_DIR",
            ),
            ("TARGET_TEMP_DIR", "<PROJECT_DIR>/rel/TARGET_TEMP_DIR"),
            ("TEMP_DIR", "<PROJECT_DIR>/rel/TEMP_DIR"),
            (
                "SHARED_PRECOMPS_DIR",
                "<PROJECT_DIR>/rel/SHARED_PRECOMPS_DIR",
            ),
            ("INSTALL_ROOT", "<PROJECT_DIR>/rel/DSTROOT"),
            ("INSTALL_DIR", "rel/INSTALL_DIR"),
            ("LOCROOT", "<PROJECT_DIR>/rel/LOCROOT"),
            ("LOCSYMROOT", "<PROJECT_DIR>/rel/LOCSYMROOT"),
            (
                "SHARED_DERIVED_FILE_DIR",
                "<PROJECT_DIR>/rel/BUILT_PRODUCTS_DIR/DerivedSources",
            ),
            (
                "DERIVED_FILE_DIR",
                "<PROJECT_DIR>/rel/TARGET_TEMP_DIR/DerivedSources",
            ),
            (
                "OBJECT_FILE_DIR",
                "<PROJECT_DIR>/rel/TARGET_TEMP_DIR/Objects",
            ),
            ("PROJECT_TEMP_ROOT", "<PROJECT_DIR>/rel/OBJROOT"),
            (
                "DWARF_DSYM_FOLDER_PATH",
                "<PROJECT_DIR>/rel/CONFIGURATION_BUILD_DIR",
            ),
            // Built from the `TARGET_BUILD_DIR` as given, not as reported.
            (
                "CODESIGNING_FOLDER_PATH",
                "a/../rel/TARGET_BUILD_DIR/Scratch",
            ),
        ],
    },
    Case {
        name: "an -xcconfig in another directory",
        settings: &[],
        xcconfig: Some(
            "SYMROOT = $(SRCROOT)/../x/sym\n\
             OBJROOT = /tmp/../tmp/xc/obj\n\
             DSTROOT = /tmp/./xc/../xc/dst\n\
             SHARED_PRECOMPS_DIR = rel/../pch\n\
             LOCROOT = a/../loc\n\
             CONFIGURATION_BUILD_DIR = $(BUILD_DIR)/../cbd/$(CONFIGURATION)\n\
             MYPATH = /tmp/../tmp/my\n",
        ),
        expected: &[
            ("SYMROOT", "<PARENT>/x/sym"),
            ("OBJROOT", "/tmp/xc/obj"),
            ("DSTROOT", "/tmp/xc/dst"),
            ("BUILD_DIR", "<PARENT>/x/sym"),
            ("BUILD_ROOT", "<PARENT>/x/sym"),
            ("CONFIGURATION_BUILD_DIR", "<PARENT>/x/cbd/Debug"),
            ("BUILT_PRODUCTS_DIR", "<PARENT>/x/cbd/Debug"),
            ("TARGET_BUILD_DIR", "<PARENT>/x/cbd/Debug"),
            ("TEMP_ROOT", "/tmp/xc/obj"),
            ("PROJECT_TEMP_DIR", "/tmp/xc/obj/Scratch.build"),
            ("CONFIGURATION_TEMP_DIR", "/tmp/xc/obj/Scratch.build/Debug"),
            (
                "TARGET_TEMP_DIR",
                "/tmp/xc/obj/Scratch.build/Debug/Scratch.build",
            ),
            ("TEMP_DIR", "/tmp/xc/obj/Scratch.build/Debug/Scratch.build"),
            ("SHARED_PRECOMPS_DIR", "<PROJECT_DIR>/pch"),
            ("INSTALL_ROOT", "/tmp/xc/dst"),
            ("INSTALL_DIR", "/tmp/xc/dst/usr/local/bin"),
            ("LOCROOT", "<PROJECT_DIR>/loc"),
            ("LOCSYMROOT", "<PROJECT_DIR>"),
            (
                "SHARED_DERIVED_FILE_DIR",
                "<PARENT>/x/cbd/Debug/DerivedSources",
            ),
            ("DWARF_DSYM_FOLDER_PATH", "<PARENT>/x/cbd/Debug"),
            ("CODESIGNING_FOLDER_PATH", "<PARENT>/x/cbd/Debug/Scratch"),
            ("MYPATH", "/tmp/../tmp/my"),
        ],
    },
    Case {
        name: "slashes, a root's parent, a leading .. and .",
        settings: &[
            "SYMROOT=/tmp//x/",
            "OBJROOT=../../up/obj",
            "TARGET_BUILD_DIR=../rel/./x/",
            "INSTALL_DIR=/../../a",
            "DSTROOT=.",
        ],
        xcconfig: None,
        expected: &[
            ("SYMROOT", "/tmp/x"),
            ("OBJROOT", "<GRANDPARENT>/up/obj"),
            ("DSTROOT", "<PROJECT_DIR>"),
            ("BUILD_DIR", "/tmp/x"),
            ("BUILD_ROOT", "/tmp/x"),
            ("CONFIGURATION_BUILD_DIR", "/tmp/x/Debug"),
            ("BUILT_PRODUCTS_DIR", "/tmp/x/Debug"),
            ("TARGET_BUILD_DIR", "../rel/x"),
            ("TEMP_ROOT", "<GRANDPARENT>/up/obj"),
            (
                "TARGET_TEMP_DIR",
                "<GRANDPARENT>/up/obj/Scratch.build/Debug/Scratch.build",
            ),
            ("INSTALL_ROOT", "<PROJECT_DIR>"),
            ("INSTALL_DIR", "/a"),
            ("SHARED_DERIVED_FILE_DIR", "/tmp/x/Debug/DerivedSources"),
            ("DWARF_DSYM_FOLDER_PATH", "/tmp/x/Debug"),
            ("CODESIGNING_FOLDER_PATH", "../rel/./x//Scratch"),
        ],
    },
    Case {
        name: "a relative SYMROOT",
        settings: &["SYMROOT=build"],
        xcconfig: None,
        expected: &[
            ("SYMROOT", "<PROJECT_DIR>/build"),
            ("BUILD_DIR", "<PROJECT_DIR>/build"),
            ("BUILD_ROOT", "<PROJECT_DIR>/build"),
            ("CONFIGURATION_BUILD_DIR", "<PROJECT_DIR>/build/Debug"),
            ("BUILT_PRODUCTS_DIR", "<PROJECT_DIR>/build/Debug"),
            ("TARGET_BUILD_DIR", "<PROJECT_DIR>/build/Debug"),
            ("DWARF_DSYM_FOLDER_PATH", "<PROJECT_DIR>/build/Debug"),
            (
                "CODESIGNING_FOLDER_PATH",
                "<PROJECT_DIR>/build/Debug/Scratch",
            ),
            ("LOCROOT", "<PROJECT_DIR>"),
        ],
    },
];

/// `template` with the directory placeholders filled in from `project_dir`.
fn expand(template: &str, project_dir: &Path) -> String {
    let parent = project_dir.parent().expect("project dir has a parent");
    let grandparent = parent.parent().expect("parent has a parent");
    template
        .replace("<PROJECT_DIR>", &project_dir.display().to_string())
        .replace("<GRANDPARENT>", &grandparent.display().to_string())
        .replace("<PARENT>", &parent.display().to_string())
}

#[test]
fn build_locations_fold_the_way_xcodebuild_reports_them() {
    let catalog = CatalogCache::new().get("27.0.0").clone();
    let scratch = ScratchDir::new("sweetpad-fold-oracle").expect("scratch dir");
    let mut failures = Vec::new();
    for case in CASES {
        let mut ctx = BuildContext::open(&scratch_project())
            .expect("open Scratch")
            .with_xcspec(catalog.clone());
        if let Some(body) = case.xcconfig {
            let path = scratch.join("fold.xcconfig");
            std::fs::write(&path, body).expect("write xcconfig");
            ctx = ctx.with_extra_xcconfig(&path).expect("layer xcconfig");
        }
        let mut query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64");
        for setting in case.settings {
            let (key, value) = setting.split_once('=').expect("KEY=VALUE");
            query = query.with_override(key, value);
        }
        let settings = ctx.resolve(&query).expect("resolve").settings;
        let project_dir = PathBuf::from(&settings["PROJECT_DIR"]);
        for (key, template) in case.expected {
            let want = expand(template, &project_dir);
            let got = settings.get(*key).map_or("<unset>", String::as_str);
            if got != want {
                failures.push(format!(
                    "{}: {key}\n  xcodebuild {want}\n  resolver   {got}",
                    case.name
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
