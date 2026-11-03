import AppKit
import WebKit

final class AuditCollectionItem: NSCollectionViewItem {
    override func loadView() { view = NSView(frame: NSRect(x: 0, y: 0, width: 50, height: 50)) }
}
final class AuditCollectionSource: NSObject, NSCollectionViewDataSource {
    func collectionView(_ collectionView: NSCollectionView, numberOfItemsInSection section: Int) -> Int { 3 }
    func collectionView(_ collectionView: NSCollectionView, itemForRepresentedObjectAt indexPath: IndexPath) -> NSCollectionViewItem {
        collectionView.makeItem(withIdentifier: NSUserInterfaceItemIdentifier("AuditItem"), for: indexPath)
    }
}
final class AuditOutlineSource: NSObject, NSOutlineViewDataSource {
    let parentNode = NSObject()
    let childNode = NSObject()
    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int { item == nil || item as AnyObject? === parentNode ? 1 : 0 }
    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any { item == nil ? parentNode : childNode }
    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool { item as AnyObject === parentNode }
    func outlineView(_ outlineView: NSOutlineView, objectValueFor tableColumn: NSTableColumn?, byItem item: Any?) -> Any? { "Node" }
}
final class AuditTableSource: NSObject, NSTableViewDataSource {
    func numberOfRows(in tableView: NSTableView) -> Int { 3 }
    func tableView(_ tableView: NSTableView, objectValueFor tableColumn: NSTableColumn?, row: Int) -> Any? { "Row \(row)" }
}
final class AuditWindowController: NSWindowController {}
final class AuditController: NSViewController {
    @IBOutlet weak var button: NSButton!
    @IBOutlet var defaults: NSUserDefaultsController!
    var lastSegue: NSStoryboardSegue?
    override func prepare(for segue: NSStoryboardSegue, sender: Any?) { lastSegue = segue }
    var presses = 0
    @objc func press(_ sender: Any?) { presses += 1 }
}
final class AuditField: NSTextField {}
final class AuditTextView: NSTextView {}
final class AuditOwner: NSObject {
    @objc dynamic var message = "Bound message"
    @objc func press(_ sender: Any?) {}
}
final class AuditCorpusOwner: NSObject {
    @IBOutlet var view: NSView!
    @IBOutlet weak var folderPopUpButton: NSPopUpButton!
    @IBOutlet weak var nameTextField: NSTextField!
    @objc func send(_ sender: Any?) {}
    @objc func cancel(_ sender: Any?) {}
    override func setValue(_ value: Any?, forUndefinedKey key: String) {}
}
func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
func check(_ condition: @autoclosure () -> Bool, _ name: String) {
    let passed = condition()
    print("IBMACAUDIT \(name)=\(passed)")
    if !passed { exit(1) }
}
func colorComponents(_ color: NSColor?) -> [Double] {
    guard let color = color?.usingColorSpace(.sRGB) else { return [] }
    return [color.redComponent, color.greenComponent, color.blueComponent, color.alphaComponent].map(Double.init)
}
func load(_ name: String, owner: Any?) -> [Any] {
    var objects: NSArray?
    check(NSNib(nibNamed: name, bundle: .main)!.instantiate(withOwner: owner, topLevelObjects: &objects), "load-\(name)")
    return objects!.map { $0 }
}
let app = NSApplication.shared
let cellObjects = load("MacCell", owner: nil)
let item = cellObjects.compactMap { $0 as? NSCollectionViewItem }.first!
check(item.view.frame.size == NSSize(width: 250, height: 250), "collection-view-item")
check(item.imageView?.superview === item.view, "image-outlet")
check(item.view.constraints.count == 4, "constraints")
let controls = load("MacControls", owner: nil).compactMap { $0 as? NSView }.first!.subviews
let field = controls.compactMap { $0 as? NSTextField }.first!
check(field.stringValue == "Hello" && field.font?.pointSize == 13, "text-field")
let button = controls.compactMap { $0 as? NSButton }.first!
check(button.title == "Press" && button.bezelStyle == .rounded, "button")
let progress = controls.compactMap { $0 as? NSProgressIndicator }.first!
check(progress.minValue == 0 && progress.maxValue == 0 && !progress.isIndeterminate, "progress-defaults")
let owner = AuditOwner()
let checkButtons = load("MacCheckButtons", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSButton }
check(checkButtons.count == 4 && checkButtons[0].state == .on && checkButtons[1].state == .mixed && checkButtons[2].state == .off, "check-button-states")
check(checkButtons[1].allowsMixedState && !checkButtons[0].allowsMixedState, "mixed-state-button")
check(checkButtons.allSatisfy { $0.alignment == .natural }, "button-default-alignment")
checkButtons[0].performClick(nil)
check(checkButtons[0].state == .off, "checkbox-toggle")
checkButtons[2].performClick(nil)
checkButtons[2].performClick(nil)
check(checkButtons[2].state == .on, "radio-stays-selected")
let controlPriorities = load("MacControlPriorities", owner: nil).compactMap { $0 as? NSView }.first!.subviews
let priorityButtons = controlPriorities.compactMap { $0 as? NSButton }
check(priorityButtons.map { $0.contentHuggingPriority(for: .vertical).rawValue } == [750, 250, 750, 750], "button-bezel-priorities")
print("IBMACOBSERVE text-field-completion=\(controlPriorities.compactMap { $0 as? NSTextField }.first!.isAutomaticTextCompletionEnabled)")
for name in ["MacViewController", "MacUnnamedController", "MacConnectedController", "MacMenuAndController"] {
    let storyboard = NSStoryboard(name: name, bundle: .main)
    let controller = storyboard.instantiateInitialController() as! NSViewController
    check(controller.view.frame.size == NSSize(width: 300, height: 200) && controller.storyboard === storyboard, "storyboard-view-loading-" + name)
    let identifier = name == "MacUnnamedController" ? "NSViewController-vcA-aa-Aud" : "AuditController"
    let another = storyboard.instantiateController(withIdentifier: NSStoryboard.SceneIdentifier(identifier)) as! NSViewController
    check(another !== controller && another.view.frame.size == controller.view.frame.size, "storyboard-named-controller-" + name)
    if name == "MacConnectedController" {
        let custom = controller as! AuditController
        check(custom.button === custom.view.subviews.first && custom.button.target === custom && custom.button.action == #selector(AuditController.press(_:)), "storyboard-outlet-and-action")
        custom.button.performClick(nil)
        check(custom.presses == 1, "storyboard-button-action")
    }
}
for name in ["MacWindowController", "MacCustomWindowController", "MacWindowRelationship", "MacCustomWindowRelationship"] {
    let storyboard = NSStoryboard(name: name, bundle: .main)
    let controller = storyboard.instantiateInitialController() as! NSWindowController
    let window = controller.window!
    check(controller.storyboard === storyboard && window.title == "Inspector", "storyboard-window-loading-" + name)
    check(!window.isVisible && window.identifier?.rawValue == "InspectorPanel" && window.tabbingMode == .disallowed, "storyboard-window-properties-" + name)
    check(window.contentMinSize == NSSize(width: 256, height: 256) && window.contentMaxSize == NSSize(width: 256, height: 2048), "storyboard-window-size-limits-" + name)
    check((controller is AuditWindowController) == name.contains("Custom"), "storyboard-window-class-" + name)
    if name.contains("Relationship") {
        let content = window.contentViewController as! AuditController
        check(content.storyboard === storyboard && content.view === window.contentView && content.button.target === content, "storyboard-window-content-" + name)
        content.button.performClick(nil)
        check(content.presses == 1, "storyboard-window-content-action-" + name)
    } else { check(window.contentViewController == nil, "storyboard-window-empty-content-" + name) }
}
let split = NSStoryboard(name: "MacSplitController", bundle: .main).instantiateInitialController() as! NSSplitViewController
let splitWindow = NSWindow(contentViewController: split)
check(split.splitViewItems.count == 2 && split.children.map { $0.title ?? "" } == ["Pane 1", "Pane 2"], "split-controller-children")
check(split.splitView.isVertical && split.splitView.dividerStyle == .thin && split.splitView.delegate === split, "split-controller-view")
check(split.splitViewItems[0].canCollapse && !split.splitViewItems[1].canCollapse && split.splitViewItems[0].holdingPriority.rawValue == 260 && split.splitViewItems[1].holdingPriority.rawValue == 250, "split-item-priorities")
print("IBMACOBSERVE split-frames=\(split.splitViewItems.map { $0.viewController.view.frame })")
split.splitViewItems[0].isCollapsed = true
check(split.splitViewItems[0].isCollapsed, "split-item-collapse")
splitWindow.orderOut(nil)
let embeddedController = NSStoryboard(name: "MacEmbedSplit", bundle: .main).instantiateInitialController() as! NSViewController
_ = embeddedController.view
check(embeddedController.children.count == 1 && embeddedController.children.first is NSSplitViewController && embeddedController.children.first!.view.superview === embeddedController.view.subviews.first, "embedded-split-controller")
let outlineSource = AuditOutlineSource()
for name in ["MacOutlineView", "MacOutlineSidebar", "MacOutlineOptions"] {
    let scroll = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSScrollView }.first!
    let outline = scroll.documentView as! NSOutlineView
    check(outline.indentationPerLevel == (name == "MacOutlineView" ? 0 : name == "MacOutlineSidebar" ? 13 : 22), "outline-indentation-" + name)
    check(outline.rowHeight == (name == "MacOutlineView" ? 17 : name == "MacOutlineSidebar" ? 32 : 25), "outline-row-height-" + name)
    outline.dataSource = outlineSource
    outline.reloadData()
    check(outline.numberOfRows == 1, "outline-data-source-" + name)
    outline.expandItem(outlineSource.parentNode)
    check(outline.numberOfRows == 2 && outline.parent(forItem: outlineSource.childNode) as AnyObject? === outlineSource.parentNode, "outline-expand-" + name)
}
let recessed = load("MacRecessedButton", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSButton }.first!
check(recessed.bezelStyle == .recessed, "recessed-button")
let sceneDefaults = NSUserDefaultsController.shared
(sceneDefaults.values as! NSObject).setValue("Scene first", forKey: "AuditSceneMessage")
let sceneObjectController = NSStoryboard(name: "MacSceneObjects", bundle: .main).instantiateInitialController() as! AuditController
let sceneField = sceneObjectController.view.subviews.compactMap { $0 as? NSTextField }.first!
check(sceneObjectController.defaults === sceneDefaults && sceneField.stringValue == "Scene first", "scene-model-outlet-and-binding")
(sceneDefaults.values as! NSObject).setValue("Scene updated", forKey: "AuditSceneMessage")
check(sceneField.stringValue == "Scene updated", "scene-model-observation")
(sceneDefaults.values as! NSObject).setValue(nil, forKey: "AuditSceneMessage")
let unnamedWindow = NSStoryboard(name: "MacUnnamedWindowContent", bundle: .main).instantiateInitialController() as! NSWindowController
check(unnamedWindow.contentViewController is AuditController && (unnamedWindow.contentViewController as! AuditController).button != nil, "unnamed-window-content-controller")
let showController = NSStoryboard(name: "MacShowSegue", bundle: .main).instantiateInitialController() as! AuditController
let showWindow = NSWindow(contentViewController: showController)
showController.button.performClick(nil)
check(showController.lastSegue != nil && (showController.lastSegue?.destinationController as? NSViewController)?.view.frame.size == NSSize(width: 300, height: 200), "mac-show-segue-action")
showWindow.orderOut(nil)
for name in ["MacCollectionView", "MacCollectionOptions", "MacCollectionDefaults", "MacLegacyCollection"] {
    let collection = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSCollectionView }.first!
    check(collection.backgroundColors.count == 1, "collection-background-" + name)
    if name != "MacLegacyCollection" {
        let flow = collection.collectionViewLayout as! NSCollectionViewFlowLayout
        if name == "MacCollectionOptions" {
            check(collection.isSelectable && collection.allowsMultipleSelection && !collection.allowsEmptySelection, "collection-selection-flags")
            check(flow.minimumLineSpacing == 17 && flow.minimumInteritemSpacing == 23 && flow.scrollDirection == .horizontal, "collection-flow-spacing")
            check(flow.headerReferenceSize == NSSize(width: 25, height: 35) && flow.footerReferenceSize == NSSize(width: 45, height: 55), "collection-supplementary-sizes")
        } else { check(!collection.isSelectable && !collection.allowsMultipleSelection, "collection-selection-defaults-" + name) }
        check(flow.itemSize == NSSize(width: name == "MacCollectionDefaults" ? 50 : 250, height: name == "MacCollectionDefaults" ? 50 : 250), "collection-item-size-" + name)
        let source = AuditCollectionSource()
        collection.register(AuditCollectionItem.self, forItemWithIdentifier: NSUserInterfaceItemIdentifier("AuditItem"))
        collection.dataSource = source
        collection.reloadData()
        check(collection.numberOfSections == 1 && collection.numberOfItems(inSection: 0) == 3, "collection-data-source-" + name)
        check(collection.makeItem(withIdentifier: NSUserInterfaceItemIdentifier("AuditItem"), for: IndexPath(item: 0, section: 0)) is AuditCollectionItem, "collection-reuse-" + name)

    }
}
let prototypeScroll = load("MacTablePrototype", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSScrollView }.first!
let prototypeTable = prototypeScroll.documentView as! NSTableView
let prototypeFirst = prototypeTable.makeView(withIdentifier: NSUserInterfaceItemIdentifier("AuditCell"), owner: nil) as! NSTableCellView
let prototypeSecond = prototypeTable.makeView(withIdentifier: NSUserInterfaceItemIdentifier("AuditCell"), owner: nil) as! NSTableCellView
check(prototypeFirst !== prototypeSecond && prototypeFirst.textField?.stringValue == "Hello" && prototypeFirst.textField?.superview === prototypeFirst, "table-reusable-cell-nibs")
check(prototypeFirst.frame.size == NSSize(width: 157, height: 17), "table-reusable-cell-size")
let separator = load("MacSeparator", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSBox }.first!
check(separator.boxType == .separator && separator.contentViewMargins == NSSize(width: 5, height: 5), "separator-box")
let menuPackage = Bundle.main.url(forResource: "MacMenuStoryboard", withExtension: "storyboardc")!
let menuInfo = try! PropertyListSerialization.propertyList(from: Data(contentsOf: menuPackage.appendingPathComponent("Info.plist")), format: nil) as! [String: Any]
check(menuInfo["NSStoryboardMainMenu"] as? String == "MainMenu" && menuInfo["NSStoryboardVersion"] as? Int == 1, "mac-storyboard-info")
var storyboardObjects: NSArray?
let menuData = try! Data(contentsOf: menuPackage.appendingPathComponent("MainMenu.nib"))
check(NSNib(nibData: menuData, bundle: .main).instantiate(withOwner: nil, topLevelObjects: &storyboardObjects), "load-mac-storyboard-menu")
let storyboardMenu = storyboardObjects!.compactMap { $0 as? NSMenu }.first { $0.title == "Audit" }!
check(storyboardMenu.items.first?.submenu?.items.first?.title == "Run", "mac-storyboard-menu")
for name in ["MacDefaultsShared", "MacDefaultsPrivate", "MacDefaultsDeferred"] {
    let defaults = load(name, owner: nil).compactMap { $0 as? NSUserDefaultsController }.first!
    check((defaults === NSUserDefaultsController.shared) == (name == "MacDefaultsShared") && defaults.appliesImmediately == (name != "MacDefaultsDeferred"), "defaults-controller-" + name)
}
for name in ["MacTextView", "MacTextViewVariants", "MacTextViewClearBackground", "MacTextViewCalibratedColor"] {
    let text = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextView }.first!
    check(text.string.isEmpty && text.textContainer?.textView === text && text.layoutManager?.textStorage === text.textStorage, "text-system-" + name)
    check(text.minSize == NSSize(width: 180, height: 100) && text.maxSize == NSSize(width: 180, height: 10000000), "text-size-limits-" + name)
    let linkAttributes = text.linkTextAttributes ?? [:]
    check((linkAttributes[.underlineStyle] as? Int) == 1 && (linkAttributes[.cursor] as? NSCursor) === NSCursor.pointingHand, "text-link-attributes-" + name)
    if name == "MacTextViewClearBackground" || name == "MacTextViewVariants" { check(text.backgroundColor.alphaComponent == 0, "text-clear-background-" + name) }
    if name == "MacTextViewCalibratedColor" { check(abs(text.backgroundColor.alphaComponent - 0.3) < 0.0001, "text-calibrated-color") }
    if name == "MacTextViewVariants" {
        check(text is AuditTextView && !text.isEditable && text.isSelectable && !text.isRichText && !text.importsGraphics && !text.drawsBackground, "text-editing-flags")
        check(text.isHorizontallyResizable && text.isVerticallyResizable && text.allowsUndo && text.smartInsertDeleteEnabled && text.usesFindBar, "text-behavior-flags")
        // System preferences can override automatic correction defaults.
        print("IBMACOBSERVE automatic-text-options=\([text.isAutomaticSpellingCorrectionEnabled, text.isAutomaticQuoteSubstitutionEnabled, text.isAutomaticDashSubstitutionEnabled, text.isAutomaticTextReplacementEnabled, text.isAutomaticLinkDetectionEnabled, text.isAutomaticDataDetectionEnabled, text.isAutomaticTextCompletionEnabled, text.allowsCharacterPickerTouchBarItem])")
        check(text.isContinuousSpellCheckingEnabled && text.isGrammarCheckingEnabled, "text-spelling-flags")
        print("IBMACOBSERVE text-colors=\(colorComponents(text.textColor))|\(colorComponents(text.insertionPointColor))")
        check(text.textColor != nil, "text-color-present")
    }
}
for (name, scale) in [("MacScrollView", 1.0), ("MacScrollViewVariants", 1.5), ("MacScrollZoomIn", 2.0), ("MacScrollZoomOut", 0.5)] {
    let scroll = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSScrollView }.first!
    check(scroll.documentView?.superview === scroll.contentView && scroll.documentView?.frame.size == NSSize(width: 180, height: 100), "scroll-document-" + name)
    check(!scroll.hasHorizontalScroller && !scroll.hasVerticalScroller && scroll.borderType == .noBorder && abs(scroll.magnification - scale) < 0.0001, "scroll-properties-" + name)
    check(abs(scroll.contentView.bounds.width - 180 / scale) < 0.0001 && abs(scroll.contentView.bounds.height - 100 / scale) < 0.0001, "scroll-clip-transform-" + name)
    if name != "MacScrollView" {
        check(scroll.allowsMagnification && scroll.minMagnification == 0.5 && scroll.maxMagnification == 8 && scroll.autohidesScrollers, "scroll-options-" + name)
        check(scroll.horizontalScrollElasticity == .none && scroll.verticalScrollElasticity == .allowed && scroll.horizontalLineScroll == 30 && scroll.verticalPageScroll == 25 && !scroll.usesPredominantAxisScrolling && !scroll.contentView.drawsBackground, "scroll-configuration-" + name)
    }
}
let tableSource = AuditTableSource()
for name in ["MacTableView", "MacTableViewDefaultSpacing", "MacTableColumns"] {
    let scroll = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSScrollView }.first!
    let table = scroll.documentView as! NSTableView
    let explicitSpacing = name == "MacTableView"
    check(table.enclosingScrollView === scroll && table.tableColumns.count == (name == "MacTableColumns" ? 2 : 1), "table-columns-" + name)
    check(table.rowHeight == 17 && table.intercellSpacing == NSSize(width: explicitSpacing ? 3 : 0, height: explicitSpacing ? 2 : 0), "table-row-geometry-" + name)
    check(table.allowsMultipleSelection && table.allowsEmptySelection && !table.allowsColumnSelection && table.allowsColumnReordering && table.allowsColumnResizing, "table-selection-options-" + name)
    let firstColumn = table.tableColumns.first!
    check(firstColumn.minWidth == 40 && firstColumn.maxWidth == 1000 && firstColumn.resizingMask == [.autoresizingMask, .userResizingMask], "table-column-options-" + name)
    check(firstColumn.headerCell.font?.pointSize == 11 && (firstColumn.dataCell as? NSTextFieldCell)?.stringValue == "Text Cell", "table-cells-" + name)
    print("IBMACOBSERVE table-widths-\(name)=\(table.tableColumns.map { Double($0.width) })")
    table.dataSource = tableSource
    table.reloadData()
    check(table.numberOfRows == 3 && table.rect(ofRow: 1).minY > 0, "table-data-source-" + name)
    print("IBMACOBSERVE table-row-rect-\(name)=\(table.rect(ofRow: 1))")
    table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
    check(table.selectedRow == 1, "table-row-selection-" + name)
}
for (name, widths, minimum) in [("MacTableOverflow", [100.0, 200.0], 335.0), ("MacTableMinimum", [160.0], 192.0), ("MacTableFixedColumn", [177.0], 209.0), ("MacTableSmallColumn", [100.0], 180.0)] {
    let scroll = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSScrollView }.first!
    let table = scroll.documentView as! NSTableView
    check(table.tableColumns.map { Double($0.width) } == widths && table.frame.width == minimum, "table-preview-sizing-" + name)
    if name == "MacTableFixedColumn" { check(table.tableColumns.first!.resizingMask.isEmpty, "table-fixed-column") }
}
let scrollerPriorities = load("MacScrollerPriorities", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSScrollView }.first!
check(scrollerPriorities.horizontalScroller!.contentHuggingPriority(for: .horizontal).rawValue == 250 && scrollerPriorities.horizontalScroller!.contentHuggingPriority(for: .vertical).rawValue == 750, "horizontal-scroller-default-priorities")
check(scrollerPriorities.verticalScroller!.contentHuggingPriority(for: .horizontal).rawValue == 301 && scrollerPriorities.verticalScroller!.contentHuggingPriority(for: .vertical).rawValue == 601, "vertical-scroller-custom-priorities")
let defaultPriorityViews = load("MacDefaultPriorities", owner: nil).compactMap { $0 as? NSView }.first!.subviews
check(defaultPriorityViews.allSatisfy { $0.contentHuggingPriority(for: .horizontal).rawValue == 250 && $0.contentHuggingPriority(for: .vertical).rawValue == 750 }, "default-control-priorities")
let customPriorityViews = load("MacCustomPriorities", owner: nil).compactMap { $0 as? NSView }.first!.subviews
check(customPriorityViews.allSatisfy { $0.contentHuggingPriority(for: .horizontal).rawValue == 251 && $0.contentHuggingPriority(for: .vertical).rawValue == 752 }, "custom-hugging-priorities")
check(customPriorityViews.allSatisfy { $0.contentCompressionResistancePriority(for: .horizontal).rawValue == 740 && $0.contentCompressionResistancePriority(for: .vertical).rawValue == 730 }, "compression-priorities")
let viewProperties = load("MacViewProperties", owner: nil).compactMap { $0 as? NSView }.first!
check(abs(viewProperties.alphaValue - 0.45) < 0.0001 && viewProperties.identifier?.rawValue == "audit-id" && viewProperties.toolTip == "Audit tooltip", "view-metadata")
check(viewProperties.subviews.allSatisfy { abs($0.alphaValue - 0.45) < 0.0001 && $0.identifier?.rawValue == "audit-id" && $0.toolTip == "Audit tooltip" }, "control-metadata")
let constraintButton = load("MacConstraintVariants", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSButton }.first!
let variantConstraint = constraintButton.constraints.first!
check(variantConstraint.relation == .lessThanOrEqual && variantConstraint.multiplier == 2 && variantConstraint.constant == 150, "constraint-values")
check(variantConstraint.priority.rawValue == 750 && variantConstraint.identifier == "audit-width", "constraint-metadata")
let placeholderButton = load("MacPlaceholderConstraint", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSButton }.first!
check(placeholderButton.constraints.isEmpty, "placeholder-constraint-omitted")
for name in ["MacMenus", "MacMenuVariants"] {
    let menu = load(name, owner: nil).compactMap { $0 as? NSMenu }.first!
    let submenu = menu.items.first!.submenu!
    let runItem = submenu.items.first!
    check(menu.title == "Audit" && submenu.items.count == 3 && runItem.state == .mixed, "menu-hierarchy-" + name)
    check(runItem.keyEquivalent == "r" && runItem.keyEquivalentModifierMask == [.control, .option] && runItem.target == nil && runItem.action == #selector(AuditOwner.press(_:)), "menu-action-" + name)
    if name == "MacMenuVariants" { check(runItem.isAlternate && runItem.indentationLevel == 2 && runItem.identifier?.rawValue == "audit-menu" && runItem.image != nil && runItem.toolTip == "Audit tip", "menu-item-variants") }
}
let variants = load("MacControlVariants", owner: owner).compactMap { $0 as? NSView }.first!.subviews
let edit = variants.compactMap { $0 as? NSTextField }.first!
check(edit.isEditable && edit.isSelectable && edit.drawsBackground && edit.placeholderString == "Hint", "text-field-variants")
check(edit.controlSize == .small && edit.cell?.usesSingleLineMode == true, "text-field-cell")
let action = variants.compactMap { $0 as? NSButton }.first!
check(action.target === owner && action.action == #selector(AuditOwner.press(_:)), "action")
let spin = variants.compactMap { $0 as? NSProgressIndicator }.first!
check(spin.style == .spinning && spin.isIndeterminate && spin.isHidden && !spin.isDisplayedWhenStopped, "progress-flags")
check(spin.minValue == 3 && spin.maxValue == 55 && spin.doubleValue == 3, "progress-values")
let panel = load("MacWindowVariants", owner: nil).compactMap { $0 as? NSPanel }.first!
check(panel.title == "Audit window" && !panel.isVisible && panel.styleMask.contains(.utilityWindow), "window-template")
check(panel.contentMinSize == NSSize(width: 300, height: 200) && panel.contentMaxSize == NSSize(width: 900, height: 700), "window-size-limits")
check(panel.hidesOnDeactivate && !panel.isRestorable && !panel.isReleasedWhenClosed, "window-flags")
let bound = load("MacBindings", owner: owner).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }.first!
check(bound.stringValue == "Bound message", "binding-initial")
owner.message = "Updated message"
check(bound.stringValue == "Updated message", "binding-observation")
let localized = load("MacLocalizedControls", owner: nil).compactMap { $0 as? NSView }.first!.subviews
check(localized.compactMap { $0 as? NSButton }.first?.title == "Press" && localized.compactMap { $0 as? NSTextField }.first?.stringValue == "Hello", "localized-controls")
let popup = load("MacPopup", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSPopUpButton }.first!
check(popup.itemTitles == ["Item 1", "Item 2", "Item 3"] && popup.indexOfSelectedItem == 0, "popup-items")
check(popup.contentHuggingPriority(for: .horizontal).rawValue == 250 && popup.contentHuggingPriority(for: .vertical).rawValue == 750, "popup-hugging")
let pull = load("MacPopupVariants", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSPopUpButton }.first!
check(pull.pullsDown && pull.preferredEdge == .maxY, "popup-flags")
if #available(macOS 15.0, *) { check(!pull.altersStateOfSelectedItem, "popup-selection-state") }
check(pull.item(at: 1)?.tag == 42 && pull.item(at: 1)?.isEnabled == false && pull.item(at: 2)?.isSeparatorItem == true, "menu-item-flags")
let imageItem = load("MacImages", owner: nil).compactMap { $0 as? NSCollectionViewItem }.first!
check(imageItem.imageView?.image != nil, "image-resource")
for name in ["MacStack", "MacStackDefaults", "MacStackViews", "MacStackMetadata"] {
    let stack = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSStackView }.first!
    if name == "MacStack" { check(stack.orientation == .horizontal && stack.spacing == 19 && stack.alignment == .bottom, "stack-properties") }
    if name == "MacStackDefaults" { check(stack.orientation == .vertical && stack.spacing == 8 && !stack.detachesHiddenViews, "stack-defaults") }
    if name == "MacStackViews" || name == "MacStackMetadata" {
        check(stack.arrangedSubviews.count == 2 && stack.orientation == .vertical && stack.distribution == .fillEqually, "stack-arranged-\(name)")
    }
    if name == "MacStackMetadata" { check(stack.visibilityPriority(for: stack.arrangedSubviews.first!).rawValue == 500 && stack.edgeInsets.top == 1, "stack-metadata") }
}
let customField = load("MacCustomField", owner: nil).compactMap { $0 as? NSView }.first!.subviews.first!
check(customField is AuditField && (customField as? NSTextField)?.stringValue == "Hello", "custom-field-class")
let responderButton = load("MacFirstResponder", owner: owner).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSButton }.first!
check(responderButton.target == nil && responderButton.action == #selector(AuditOwner.press(_:)), "first-responder-action")
let strings = load("MacStringValues", owner: nil).compactMap { $0 as? NSView }.first!.subviews
check(strings.compactMap { $0 as? NSTextField }.first?.stringValue == "  First line\nSecond line ☃  " && strings.compactMap { $0 as? NSButton }.first?.keyEquivalent == "\r", "xml-string-values")
for (name, size) in [("subheadline", 11.0), ("headline", 13.0), ("callout", 12.0), ("title3", 15.0), ("custom", 16.0)] {
    let field = load("MacFont-" + name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }.first!
    check(field.font?.pointSize == CGFloat(size), "font-" + name)
}
let namedFonts = load("MacNamedFonts", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }
let expectedFontNames = [".AppleSystemUIFont", ".AppleSystemUIFont", "HelveticaNeue", "HelveticaNeue", ".AppleSystemUIFont", ".AppleSystemUIFont", ".AppleSystemUIFont", ".AppleSystemUIFont", ".AppleSystemUIFontBold", ".AppleSystemUIFontMedium", ".AppleSystemUIFontThin", ".AppleSystemUIFontLight", ".AppleSystemUIFont", ".AppleSystemUIFont", ".AppleSystemUIFontHeavy", ".AppleSystemUIFontBlack", "TimesNewRomanPSMT", "Menlo-Bold"]
for (index, field) in namedFonts.enumerated() {
    check(field.font?.fontName == expectedFontNames[index] && field.font?.pointSize == 16, "named-font-\(index)")
    print("IBMACOBSERVE named-font-\(index)=\(field.font!.fontDescriptor.symbolicTraits.rawValue)")
}
for name in ["MacGrid", "MacGridDefaults", "MacGridVariants"] {
    let grid = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSGridView }.first!
    check(grid.numberOfRows == 2 && grid.numberOfColumns == 2, "grid-shape-" + name)
    if name == "MacGrid" { check(grid.rowSpacing == 7 && grid.columnSpacing == 8 && grid.xPlacement == .trailing && grid.yPlacement == .center, "grid-properties") }
    if name == "MacGridDefaults" { check(grid.rowSpacing == 6 && grid.columnSpacing == 6, "grid-defaults") }
    if name == "MacGridVariants" {
        check(grid.row(at: 0).isHidden && grid.column(at: 0).isHidden && grid.subviews.count == 1, "grid-hidden-members")
        check(grid.row(at: 0).height == 25 && grid.column(at: 0).width == 80, "grid-dimensions")
        let cell = grid.cell(atColumnIndex: 0, rowIndex: 0)
        check(cell.xPlacement == .fill && cell.yPlacement == .bottom && cell.contentView?.superview == nil, "grid-cell-placement")
    }
}
let secure = load("MacSecureField", owner: owner).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSSecureTextField }.first!
check(secure.stringValue == "Secret" && secure.placeholderString == "Hint" && secure.isEditable, "secure-text-field")
let web = load("MacWebViewVariants", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? WKWebView }.first!
check(web.customUserAgent == "MacAudit" && web.allowsMagnification && web.magnification == 1.5 && !web.allowsLinkPreview, "web-view-properties")
check(web.configuration.preferences.minimumFontSize == 14 && !web.configuration.preferences.javaScriptEnabled && !web.configuration.preferences.tabFocusesLinks, "web-view-preferences")
let redField = load("MacSystemRed", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }.first!
check(redField.textColor == .systemRed, "system-color-red")
let layoutWindow = load("MacLayoutSheet", owner: nil).compactMap { $0 as? NSWindow }.first!
let layoutViews = layoutWindow.contentView!.subviews
let layoutButtons = layoutViews.compactMap { $0 as? NSButton }
check(layoutButtons.count == 2 && layoutButtons.allSatisfy { $0.frame.width == 73 && $0.frame.height == 24 }, "layout-sheet-intrinsic-and-equal-width")
for (index, view) in layoutViews.enumerated() { print("IBMACOBSERVE layout-sheet-\(index)=\(NSStringFromRect(view.frame))") }
let progressWindow = load("MacLayoutProgress", owner: nil).compactMap { $0 as? NSWindow }.first!
let progressLabel = progressWindow.contentView!.subviews.compactMap { $0 as? NSTextField }.first!
check(progressLabel.frame.size == NSSize(width: 88, height: 19) && progressLabel.font?.pointSize == 15, "layout-centered-font-padding")
for (index, view) in progressWindow.contentView!.subviews.enumerated() { print("IBMACOBSERVE layout-progress-\(index)=\(NSStringFromRect(view.frame))") }
for name in ["MacGridMixed", "MacGridIntrinsic"] {
    let grid = load(name, owner: nil).compactMap { $0 as? NSView }.first!.subviews.first as! NSGridView
    check(grid.column(at: 0).width == CGFloat(Float.leastNormalMagnitude) && grid.row(at: 0).height == CGFloat(Float.leastNormalMagnitude), "grid-automatic-dimensions-" + name)
    check((grid.cell(atColumnIndex: 0, rowIndex: 0).contentView as? NSTextField)?.stringValue == "Hello", "grid-intrinsic-content-" + name)
}
let textFieldVariants = load("MacTextFieldVariants", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }
check(textFieldVariants.count == 4 && textFieldVariants[0].isEditable && textFieldVariants[0].isSelectable && textFieldVariants[3].cell!.usesSingleLineMode, "text-field-editable-variants")
let editableColor = load("MacTextColorEditable", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }.first!
let labelColor = load("MacTextColorLabel", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }.first!
check(editableColor.textColor == .controlTextColor && labelColor.textColor == .textColor, "text-field-color-normalization")
let ranges = load("MacProgressRanges", owner: nil).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSProgressIndicator }
check(ranges.map { $0.maxValue } == [0, 0, 100, 500], "progress-range-defaults-and-explicit-values")
let multilineWindow = load("MacLayoutMultiline", owner: nil).compactMap { $0 as? NSWindow }.first!
let multilineInput = multilineWindow.contentView!.subviews.compactMap { $0 as? NSTextField }.first { $0.isEditable }!
check(!multilineInput.cell!.usesSingleLineMode && multilineInput.frame.height == 24, "layout-editable-multiline")
let layoutScene = NSStoryboard(name: "MacSceneLayout", bundle: .main).instantiateInitialController() as! AuditController
check(layoutScene.view.subviews.compactMap { $0 as? NSTextField }.count == 2, "layout-scene-model-and-label")
for (index, field) in layoutScene.view.subviews.compactMap({ $0 as? NSTextField }).enumerated() { print("IBMACOBSERVE layout-scene-\(index)=\(NSStringFromRect(field.frame))") }
let adaptiveOwner = AuditCorpusOwner()
let adaptiveObjects = load("MacLayoutAdaptive", owner: adaptiveOwner)
check(adaptiveOwner.folderPopUpButton != nil && adaptiveOwner.nameTextField != nil && adaptiveObjects.contains { $0 as AnyObject === adaptiveOwner.view }, "adaptive-sheet-outlets")
let adaptiveWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 272, height: 176), styleMask: .borderless, backing: .buffered, defer: false)
adaptiveWindow.contentView = adaptiveOwner.view
adaptiveOwner.view.needsLayout = true
adaptiveOwner.view.layoutSubtreeIfNeeded()
check(adaptiveOwner.nameTextField.frame == NSRect(x: 62, y: 88, width: 200, height: 24), "adaptive-sheet-field-after-layout")
check(adaptiveOwner.folderPopUpButton.frame == NSRect(x: 62, y: 54, width: 200, height: 24), "adaptive-sheet-popup-after-layout")
check(adaptiveOwner.view.subviews.compactMap { $0 as? NSButton }.contains { $0.keyEquivalent == "D" && $0.keyEquivalentModifierMask.contains(.command) }, "adaptive-sheet-key-equivalent")
for (index, view) in adaptiveOwner.view.subviews.enumerated() { print("IBMACOBSERVE adaptive-sheet-\(index)=\(NSStringFromRect(view.frame))") }
let sceneTable = NSStoryboard(name: "MacSceneTable", bundle: .main).instantiateInitialController() as! NSViewController
let sceneScroll = descendants(sceneTable.view).compactMap { $0 as? NSScrollView }.first!
let sceneTableView = sceneScroll.documentView as! NSTableView
check(sceneScroll.frame.size == NSSize(width: 178, height: 198), "nested-storyboard-scroll-frame")
check(sceneTableView.style == .fullWidth && sceneTableView.allowsExpansionToolTips, "storyboard-table-style-and-expansion")
let reusableCell = sceneTableView.makeView(withIdentifier: NSUserInterfaceItemIdentifier("Cell"), owner: nil) as! NSTableCellView
check(reusableCell.imageView != nil && reusableCell.textField?.stringValue == "Table View Cell", "storyboard-table-prototype-outlets")
check(reusableCell.imageView!.animates && reusableCell.textField!.allowsExpansionToolTips, "storyboard-prototype-control-properties")
print("IBMACOBSERVE storyboard-prototype-image=\(NSStringFromRect(reusableCell.imageView!.frame))")
let recessViews = load("MacRecessedPopup", owner: nil).compactMap { $0 as? NSView }.first!.subviews
let recess = recessViews.compactMap { $0 as? NSPopUpButton }.first!
check(recess.frame.width == 72 && recess.frame.height == 18 && recess.bezelStyle == .recessed, "recessed-popup-intrinsic-layout")
check(recess.indexOfSelectedItem == 1 && recess.pullsDown, "recessed-popup-default-selection")
let recessSeparator = recessViews.compactMap { $0 as? NSBox }.first!
check(recessSeparator.frame.height == 5 && (recessSeparator.titleCell as? NSTextFieldCell)?.backgroundColor == .textBackgroundColor, "storyboard-separator-defaults")
let accessibleOutline = descendants(load("MacAccessibility", owner: nil).compactMap { $0 as? NSView }.first!).compactMap { $0 as? NSOutlineView }.first!
check(accessibleOutline.accessibilityLabel() == "Audit feeds", "accessibility-description-connector")
let precision = load("MacConstraintPrecision", owner: nil).compactMap { $0 as? NSView }.first!
check(descendants(precision).flatMap { $0.constraints }.contains { abs($0.multiplier - Double(Float(0.951613))) < 1e-14 }, "constraint-single-precision-multiplier")
check(descendants(precision).flatMap { $0.constraints }.contains { abs($0.constant - 0.951613) < 1e-14 }, "constraint-double-precision-constant")
let noOptions = load("MacBindingNoOptions", owner: owner).compactMap { $0 as? NSView }.first!.subviews.compactMap { $0 as? NSTextField }.first!
check(noOptions.stringValue == owner.message, "binding-without-options")
owner.message = "No-options update"
check(noOptions.stringValue == owner.message, "binding-without-options-updates")
print("IBMACAUDIT done")
