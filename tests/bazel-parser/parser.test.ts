import { describe, expect, test, beforeAll } from '@jest/globals';
import { readFile } from 'node:fs/promises';
import * as path from 'node:path';
import { BazelParser, BazelParserUtils } from '../../src/build/bazel/parser';
import type { BazelParseResult, BazelTarget, BazelScheme } from '../../src/build/bazel/types';

describe('BazelParser', () => {
  let xcodeprojContent: string;
  let ddIosPackageContent: string;
  let cxModuleContent: string;

  beforeAll(async () => {
    const samplesDir = path.join(__dirname, 'samples');
    
    xcodeprojContent = await readFile(path.join(samplesDir, 'xcodeproj.BUILD'), 'utf-8');
    ddIosPackageContent = await readFile(path.join(samplesDir, 'dd_ios_package.BUILD'), 'utf-8');
    cxModuleContent = await readFile(path.join(samplesDir, 'cx_module.BUILD'), 'utf-8');
  });

  describe('xcodeproj parsing', () => {
    test('should parse xcschemes from xcodeproj', () => {
      const result = BazelParser.parse(xcodeprojContent);
      
      expect(result.xcschemes).toBeDefined();
      expect(result.xcschemes.length).toBeGreaterThan(0);
      
      // Check for doordash_scheme
      const doordashScheme = result.xcschemes.find(s => s.name === 'DoorDash');
      expect(doordashScheme).toBeDefined();
      expect(doordashScheme?.type).toBe('doordash_scheme');
      expect(doordashScheme?.buildTargets).toContain('//Apps/Consumer/ConsumerApp:DoorDash');
      
      // Check for doordash_scheme with env
      const proxyScheme = result.xcschemes.find(s => s.name === 'DoorDashProxy');
      expect(proxyScheme).toBeDefined();
      expect(proxyScheme?.env?.PROXY_HOST).toBe('localhost');
      expect(proxyScheme?.env?.PROXY_PORT).toBe('8888');
      
      // Check for appclip scheme
      const appclipScheme = result.xcschemes.find(s => s.name === 'DoorDashAppClip');
      expect(appclipScheme).toBeDefined();
      expect(appclipScheme?.type).toBe('doordash_appclip_scheme');
      
      // Check for xcschemes.scheme
      const legoScheme = result.xcschemes.find(s => s.name === 'LegoDevApp');
      expect(legoScheme).toBeDefined();
      expect(legoScheme?.type).toBe('xcschemes_scheme');
      expect(legoScheme?.buildTargets).toContain('//Apps/Consumer/ConsumerApp/LegoDevApp');
      expect(legoScheme?.env?.PROXY_HOST).toBe('localhost');
    });

    test('should parse xcode_configurations', () => {
      const result = BazelParser.parse(xcodeprojContent);
      
      expect(result.xcode_configurations).toBeDefined();
      expect(result.xcode_configurations.length).toBeGreaterThan(0);
      
      const config = result.xcode_configurations[0];
      expect(config.name).toBe('xcode_configurations');
      expect(config.buildSettings?.source).toBe('//bazel_support/config:config.bzl');
    });

    test('should parse top_level_targets', () => {
      const result = BazelParser.parse(xcodeprojContent);
      
      expect(result.targets.length).toBeGreaterThan(0);
      expect(result.targetsTest.length).toBeGreaterThan(0);
      
      // Check for main app target
      const doorDashTarget = result.targets.find(t => t.name === 'DoorDash');
      expect(doorDashTarget).toBeDefined();
      expect(doorDashTarget?.type).toBe('binary');
      expect(doorDashTarget?.buildLabel).toBe('//Apps/Consumer/ConsumerApp:DoorDash');
      
      // Check for test target
      const testTarget = result.targetsTest.find(t => t.name === 'DoorDashTests');
      expect(testTarget).toBeDefined();
      expect(testTarget?.type).toBe('test');
      expect(testTarget?.buildLabel).toBe('//Apps/Consumer/ConsumerApp/Tests:DoorDashTests');
    });
  });

  describe('dd_ios_package parsing', () => {
    test('should parse library targets', () => {
      const result = BazelParser.parse(ddIosPackageContent, '/path/to/Topaz/BUILD.bazel');
      
      expect(result.targets.length).toBeGreaterThan(10);
      
      // Check specific library target
      const cachingTarget = result.targets.find(t => t.name === 'TopazCaching');
      expect(cachingTarget).toBeDefined();
      expect(cachingTarget?.type).toBe('library');
      expect(cachingTarget?.deps).toContain(':TopazDataStructures');
      expect(cachingTarget?.path).toBe('Sources/Caching');
      expect(cachingTarget?.buildLabel).toBe('//Topaz:TopazCaching');
      
      // Check target with external deps
      const dataStructuresTarget = result.targets.find(t => t.name === 'TopazDataStructures');
      expect(dataStructuresTarget).toBeDefined();
      expect(dataStructuresTarget?.deps).toContain('@swiftpkg_swift_collections//:OrderedCollections');
      
      // Check target with resources
      const testHelpersTarget = result.targets.find(t => t.name === 'TopazUnitTestHelpers');
      expect(testHelpersTarget).toBeDefined();
      expect(testHelpersTarget?.resources).toBeDefined();
      expect(testHelpersTarget?.resources?.length).toBe(4);
      expect(testHelpersTarget?.resources).toContain('Sources/UnitTestHelpers/Resources/Images/testimage.png');
    });

    test('should parse test targets', () => {
      const result = BazelParser.parse(ddIosPackageContent, '/path/to/Topaz/BUILD.bazel');
      
      expect(result.targetsTest.length).toBeGreaterThan(10);
      
      // Check specific test target
      const cachingTestTarget = result.targetsTest.find(t => t.name === 'TopazCachingTests');
      expect(cachingTestTarget).toBeDefined();
      expect(cachingTestTarget?.type).toBe('test');
      expect(cachingTestTarget?.deps).toContain(':TopazUnitTestHelpers');
      expect(cachingTestTarget?.path).toBe('Tests/CachingTests');
      expect(cachingTestTarget?.buildLabel).toBe('//Topaz:TopazCachingTests');
      expect(cachingTestTarget?.testLabel).toBe('//Topaz:TopazCachingTests');
      
      // Check test with multiple deps
      const foundationTestTarget = result.targetsTest.find(t => t.name === 'TopazFoundationExtensionsTests');
      expect(foundationTestTarget).toBeDefined();
      expect(foundationTestTarget?.deps).toContain(':TopazFoundationExtensions');
      expect(foundationTestTarget?.deps).toContain(':TopazUnitTestHelpers');
    });

    test('should separate targets and test targets correctly', () => {
      const result = BazelParser.parse(ddIosPackageContent);
      
      // All targets should be libraries
      result.targets.forEach(target => {
        expect(target.type).toBe('library');
        expect(target.name.includes('Tests')).toBe(false);
      });
      
      // All test targets should be tests
      result.targetsTest.forEach(target => {
        expect(target.type).toBe('test');
        expect(target.name.includes('Tests')).toBe(true);
        expect(target.testLabel).toBeDefined();
      });
    });
  });

  describe('cx_module parsing', () => {
    test('should create default targets for cx_module', () => {
      const result = BazelParser.parse(cxModuleContent, '/Apps/Consumer/SomeModule/BUILD.bazel');
      
      expect(result.targets.length).toBe(1);
      expect(result.targetsTest.length).toBe(1);
      
      // Check library target
      const libraryTarget = result.targets[0];
      expect(libraryTarget.name).toBe('SomeModule');
      expect(libraryTarget.type).toBe('library');
      expect(libraryTarget.buildLabel).toBe('//Apps/Consumer/SomeModule:SomeModule');
      
      // Check test target
      const testTarget = result.targetsTest[0];
      expect(testTarget.name).toBe('SomeModuleTests');
      expect(testTarget.type).toBe('test');
      expect(testTarget.deps).toContain(':SomeModule');
      expect(testTarget.buildLabel).toBe('//Apps/Consumer/SomeModule:SomeModuleTests');
      expect(testTarget.testLabel).toBe('//Apps/Consumer/SomeModule:SomeModuleTests');
    });
  });

  describe('parsePackage', () => {
    test('should parse complete package info', () => {
      const packageInfo = BazelParser.parsePackage(
        ddIosPackageContent, 
        '/path/to/Topaz'
      );
      
      expect(packageInfo.name).toBe('Topaz');
      expect(packageInfo.path).toBe('/path/to/Topaz');
      expect(packageInfo.parseResult.targets.length).toBeGreaterThan(0);
      expect(packageInfo.parseResult.targetsTest.length).toBeGreaterThan(0);
    });
  });

  describe('complex scenarios', () => {
    test('should handle mixed content', () => {
      const mixedContent = `
load("@rules_xcodeproj//xcodeproj:defs.bzl", "xcodeproj")
load("//bazel_support/rules:dd_ios_package.bzl", "dd_ios_package", "target")

dd_ios_package(
    name = "MixedPackage",
    targets = [
        target.library(
            name = "MyLibrary",
            deps = [":OtherLib"],
            path = "Sources/MyLibrary",
        ),
        target.test(
            name = "MyLibraryTests",
            deps = [":MyLibrary"],
            path = "Tests/MyLibraryTests",
        ),
    ],
)

xcodeproj(
    name = "test_project",
    xcschemes = [
        xcschemes.scheme(
            name = "TestScheme",
            run = xcschemes.run(
                build_targets = ["//Mixed:MyLibrary"],
                launch_target = "//Mixed:MyLibrary",
            ),
        ),
    ],
)
`;
      
      const result = BazelParser.parse(mixedContent);
      
      expect(result.targets.length).toBe(1);
      expect(result.targetsTest.length).toBe(1);
      expect(result.xcschemes.length).toBe(1);
      
      const scheme = result.xcschemes[0];
      expect(scheme.name).toBe('TestScheme');
      expect(scheme.buildTargets).toContain('//Mixed:MyLibrary');
    });

    test('should handle empty content', () => {
      const result = BazelParser.parse('');
      
      expect(result.targets).toEqual([]);
      expect(result.targetsTest).toEqual([]);
      expect(result.xcschemes).toEqual([]);
      expect(result.xcode_configurations).toEqual([]);
    });

    test('should handle malformed content gracefully', () => {
      const malformedContent = `
      dd_ios_package(
        name = "Incomplete"
        targets = [
          target.library(
            name = "Broken
      `;
      
      const result = BazelParser.parse(malformedContent);
      
      // Should not crash and return empty results
      expect(result.targets).toEqual([]);
      expect(result.targetsTest).toEqual([]);
    });
  });
});

describe('BazelParserUtils', () => {
  describe('extractStringArray', () => {
    test('should extract string arrays', () => {
      const content = '["first", "second", "third"]';
      const result = BazelParserUtils.extractStringArray(content);
      
      expect(result).toEqual(['first', 'second', 'third']);
    });

    test('should handle empty arrays', () => {
      const content = '[]';
      const result = BazelParserUtils.extractStringArray(content);
      
      expect(result).toEqual([]);
    });
  });

  describe('extractDict', () => {
    test('should extract key-value pairs', () => {
      const content = '"key1": "value1", "key2": "value2"';
      const result = BazelParserUtils.extractDict(content);
      
      expect(result).toEqual({
        key1: 'value1',
        key2: 'value2'
      });
    });

    test('should handle empty dicts', () => {
      const content = '';
      const result = BazelParserUtils.extractDict(content);
      
      expect(result).toEqual({});
    });
  });

  describe('findBalancedParens', () => {
    test('should find balanced parentheses', () => {
      const content = 'func(param1, func2(nested))';
      const result = BazelParserUtils.findBalancedParens(content, 4);
      
      expect(result).toBe('param1, func2(nested)');
    });

    test('should handle unbalanced parentheses', () => {
      const content = 'func(param1, func2(nested';
      const result = BazelParserUtils.findBalancedParens(content, 4);
      
      expect(result).toBeNull();
    });
  });
});

describe('Integration Tests', () => {
  test('should parse all sample files without errors', async () => {
    const samplesDir = path.join(__dirname, 'samples');
    const files = ['xcodeproj.BUILD', 'dd_ios_package.BUILD', 'cx_module.BUILD'];
    
    for (const file of files) {
      const content = await readFile(path.join(samplesDir, file), 'utf-8');
      const result = BazelParser.parse(content, `/path/to/${file}`);
      
      // Each file should produce valid results
      expect(result).toBeDefined();
      expect(typeof result.targets).toBe('object');
      expect(typeof result.targetsTest).toBe('object');
      expect(typeof result.xcschemes).toBe('object');
      expect(typeof result.xcode_configurations).toBe('object');
    }
  });

  test('should handle real-world complex BUILD files', () => {
    const complexContent = `
load("@rules_xcodeproj//xcodeproj:defs.bzl", "top_level_target", "xcodeproj", "xcschemes")
load("//bazel_support/config:config.bzl", "xcode_configurations")
load("//bazel_support/rules:dd_ios_package.bzl", "dd_ios_package", "target")
load("//Apps/Consumer:cx_module.bzl", "cx_module")

# Multiple package types in one file
dd_ios_package(
    name = "Core",
    targets = [
        target.library(
            name = "CoreLib",
            deps = ["@external//:SomeDep"],
            path = "Sources/Core",
        ),
        target.test(
            name = "CoreTests",
            deps = [":CoreLib"],
            path = "Tests/CoreTests",
        ),
    ],
)

cx_module(
    features = ["swift.upcoming.BareSlashRegexLiterals"]
)

xcodeproj(
    name = "ComplexProject",
    top_level_targets = [
        top_level_target("//Complex:CoreLib"),
        top_level_target("//Complex:App"),
    ],
    xcschemes = [
        xcschemes.scheme(
            name = "ComplexApp",
            run = xcschemes.run(
                build_targets = ["//Complex:App"],
                launch_target = "//Complex:App",
                env = {"DEBUG": "1", "ENV": "development"},
            ),
            test = xcschemes.test(
                test_targets = ["//Complex:CoreTests"],
            ),
        ),
    ],
    xcode_configurations = xcode_configurations,
)
`;

    const result = BazelParser.parse(complexContent, '/Complex/BUILD.bazel');
    
    // Should parse all different rule types
    expect(result.targets.length).toBeGreaterThanOrEqual(3); // CoreLib + cx_module + top_level
    expect(result.targetsTest.length).toBeGreaterThanOrEqual(2); // CoreTests + cx_module test
    expect(result.xcschemes.length).toBe(1);
    expect(result.xcode_configurations.length).toBe(1);
    
    // Check specific parsed elements
    const coreLib = result.targets.find(t => t.name === 'CoreLib');
    expect(coreLib?.deps).toContain('@external//:SomeDep');
    
    const scheme = result.xcschemes[0];
    expect(scheme.name).toBe('ComplexApp');
    expect(scheme.env?.DEBUG).toBe('1');
  });

  test('should correctly generate package paths for Packages directory structure', () => {
    const packageContent = `
dd_ios_package(
    name = "DoordashAttestation",
    targets = [
        target.library(
            name = "DoordashAttestation",
            path = "Sources/DoordashAttestation",
        ),
        target.test(
            name = "DoordashAttestationTests",
            deps = [":DoordashAttestation"],
            path = "Tests/DoordashAttestationTests",
        ),
    ],
)
`;

    const result = BazelParser.parse(packageContent, '/workspace/Packages/DoordashAttestation/BUILD.bazel');
    
    // Check that the build labels include the full Packages/ path
    const libraryTarget = result.targets.find(t => t.name === 'DoordashAttestation');
    expect(libraryTarget).toBeDefined();
    expect(libraryTarget?.buildLabel).toBe('//Packages/DoordashAttestation:DoordashAttestation');
    
    const testTarget = result.targetsTest.find(t => t.name === 'DoordashAttestationTests');
    expect(testTarget).toBeDefined();
    expect(testTarget?.buildLabel).toBe('//Packages/DoordashAttestation:DoordashAttestationTests');
    expect(testTarget?.testLabel).toBe('//Packages/DoordashAttestation:DoordashAttestationTests');
  });
});
