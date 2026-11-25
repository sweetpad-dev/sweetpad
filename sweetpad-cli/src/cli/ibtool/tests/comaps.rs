//! Regression probes for the constructs used by CoMaps. These inspect the
//! archive graph, including runtime setters and connection destinations.
//! Adapted from [Casper van Elteren’s CoMaps Linux patch](https://github.com/cvanelteren/comaps-ios-linux/blob/ddf82e9d55a3eb910893d37c3d9959b97e47a2d7/sweetpad.patch).
use super::*;

fn compile_xib(body: &str) -> Result<Vec<read::Object>, String> {
    let document = sweetpad_lib::xcscheme::parse(&format!(
        r#"<document type="com.apple.InterfaceBuilder3.CocoaTouch.XIB"><objects>{body}</objects></document>"#
    )).unwrap();
    xib::compile(
        &document,
        &Options {
            module: "Maps",
            localized: false,
        },
    )
    .map(|bytes| read::parse(&bytes))
}
fn value<'a>(object: &'a read::Object, key: &str) -> &'a Read {
    &object
        .values
        .iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("missing {key} in {object:?}"))
        .1
}
fn reference(object: &read::Object, key: &str) -> usize {
    match value(object, key) {
        Read::Object(id) => *id,
        other => panic!("not a reference: {other:?}"),
    }
}
fn string(object: &read::Object) -> &str {
    match value(object, "NS.bytes") {
        Read::Data(bytes) => std::str::from_utf8(bytes).unwrap(),
        other => panic!("not a string: {other:?}"),
    }
}
fn class<'a>(objects: &'a [read::Object], name: &str) -> &'a read::Object {
    objects
        .iter()
        .find(|o| o.class == name)
        .unwrap_or_else(|| panic!("no {name}"))
}
fn dictionary(objects: &[read::Object], id: usize) -> BTreeMap<String, usize> {
    let refs: Vec<_> = objects[id]
        .values
        .iter()
        .filter_map(|(key, value)| match value {
            Read::Object(id) if key == nib::ELEMENT => Some(*id),
            _ => None,
        })
        .collect();
    refs.chunks_exact(2)
        .map(|pair| (string(&objects[pair[0]]).to_owned(), pair[1]))
        .collect()
}

#[test]
fn attributed_text_preserves_font_color_paragraph_and_unicode_runs() {
    let body = r#"<textView id="text"><rect key="frame" width="100" height="40"/>
        <attributedString key="attributedText"><fragment content="Message😀"><attributes>
        <color key="NSColor" red="0" green="0" blue="0" alpha="0.54" colorSpace="custom" customColorSpace="sRGB"/>
        <font key="NSFont" size="14" name="HelveticaNeue"/>
        <paragraphStyle key="NSParagraphStyle" alignment="left" lineBreakMode="wordWrapping" baseWritingDirection="natural"/>
        </attributes></fragment></attributedString></textView>"#;
    let objects = compile_xib(body).unwrap();
    let text = class(&objects, "UITextView");
    let styled = &objects[reference(text, "UIAttributedText")];
    assert_eq!(styled.class, "NSAttributedString");
    assert_eq!(string(&objects[reference(styled, "NSString")]), "Message😀");
    let attrs = dictionary(&objects, reference(styled, "NSAttributes"));
    let font = &objects[attrs["NSFont"]];
    assert_eq!(
        string(&objects[reference(font, "UIFontName")]),
        "HelveticaNeue"
    );
    assert_eq!(value(font, "UIFontPointSize"), &Read::Double(14.0));
    assert_eq!(
        value(&objects[attrs["NSColor"]], "UIAlpha"),
        &Read::Float(0.54)
    );
    assert_eq!(
        value(&objects[attrs["NSParagraphStyle"]], "NSWritingDirection"),
        &Read::Int(-1)
    );
    assert!(
        !styled
            .values
            .iter()
            .any(|(key, _)| key == "NSAttributeInfo")
    );
    let multi = body.replace("</attributedString>", "<fragment content=\"é\"><attributes><font key=\"NSFont\" size=\"20\" name=\"Georgia\"/></attributes></fragment></attributedString>");
    let objects = compile_xib(&multi).unwrap();
    let styled = class(&objects, "NSAttributedString");
    let info = &objects[reference(styled, "NSAttributeInfo")];
    assert_eq!(value(info, "NS.bytes"), &Read::Data(vec![9, 0, 1, 1]));
    assert_eq!(
        string(&objects[reference(styled, "NSString")]),
        "Message😀é"
    );
    let bad = body.replace("<font key=", "<unsupported key=");
    assert!(
        compile_xib(&bad)
            .unwrap_err()
            .contains("unsupported text attribute")
    );
}

#[test]
fn label_and_text_view_preserve_long_utf16_runs_only_for_the_attributed_text_key() {
    let content = "🌍".repeat(65);
    let styled = format!(
        r#"<attributedString key="attributedText"><fragment content="{content}"/><fragment content="B"><attributes><font key="NSFont" size="15" name="HelveticaNeue-Bold"/></attributes></fragment></attributedString>"#
    );
    for (tag, class_name) in [("label", "UILabel"), ("textView", "UITextView")] {
        let body = format!(r#"<{tag} id="text" usesAttributedText="YES">{styled}</{tag}>"#);
        let objects = compile_xib(&body).unwrap();
        let view = class(&objects, class_name);
        let archive = &objects[reference(view, "UIAttributedText")];
        let info = &objects[reference(archive, "NSAttributeInfo")];
        assert_eq!(value(info, "NS.bytes"), &Read::Data(vec![0x82, 1, 0, 1, 1]));
        let wrong_key = body.replace(r#"key="attributedText""#, r#"key="other""#);
        assert!(
            compile_xib(&wrong_key)
                .unwrap_err()
                .contains("<attributedString>")
        );
    }
    let plain =
        format!(r#"<label id="text" text="Plain" usesAttributedText="NO">{styled}</label>"#);
    let objects = compile_xib(&plain).unwrap();
    let label = class(&objects, "UILabel");
    assert_eq!(string(&objects[reference(label, "UIText")]), "Plain");
    assert!(
        !label
            .values
            .iter()
            .any(|(key, _)| key == "UIAttributedText")
    );
}

#[test]
fn custom_collection_layout_is_swapped_and_connected() {
    for tag in ["collectionViewLayout", "collectionViewFlowLayout"] {
        let objects = compile_xib(&format!(r#"<collectionView id="collection"><rect key="frame" width="100" height="100"/>
            <{tag} key="collectionViewLayout" id="layout" customClass="TransportTransitFlowLayout" customModule="CoMaps" customModuleProvider="target">
              <userDefinedRuntimeAttributes><userDefinedRuntimeAttribute type="number" keyPath="spacing" value="12"/></userDefinedRuntimeAttributes>
            </{tag}>
            <connections><outlet property="delegate" destination="layout" id="outlet"/></connections>
        </collectionView>"#)).unwrap();
        let collection = class(&objects, "UICollectionView");
        let layout_id = reference(collection, "UICollectionLayout");
        let layout = &objects[layout_id];
        assert_eq!(layout.class, "UIClassSwapper");
        assert_eq!(
            string(&objects[reference(layout, "UIClassName")]),
            "_TtC4Maps26TransportTransitFlowLayout"
        );
        assert_eq!(
            string(&objects[reference(layout, "UIOriginalClassName")]),
            if tag == "collectionViewLayout" {
                "UICollectionViewLayout"
            } else {
                "UICollectionViewFlowLayout"
            }
        );
        let outlet = class(&objects, "UIRuntimeOutletConnection");
        assert_eq!(reference(outlet, "UIDestination"), layout_id);
        let pair = class(&objects, "UINibKeyValuePair");
        assert_eq!(reference(pair, "UIObject"), layout_id);
        assert_eq!(string(&objects[reference(pair, "UIKeyPath")]), "spacing");
    }
}

#[test]
fn date_picker_retains_mode_interval_locale_style_dates_and_actions() {
    let body = r#"<placeholder placeholderIdentifier="IBFilesOwner" id="owner"/>
      <datePicker id="picker" datePickerMode="time" minuteInterval="5" style="wheels">
      <rect key="frame" width="128" height="180"/>
      <locale key="locale" localeIdentifier="ru"/>
      <date key="minimumDate" timeIntervalSinceReferenceDate="12345"/>
      <connections><action selector="openValueChanged" destination="owner" eventType="valueChanged" id="action"/></connections>
      </datePicker>"#;
    let objects = compile_xib(body).unwrap();
    let picker = objects
        .iter()
        .position(|o| o.class == "UIDatePicker")
        .unwrap();
    let mut properties = BTreeMap::new();
    for pair in objects.iter().filter(|o| o.class == "UINibKeyValuePair") {
        assert_eq!(reference(pair, "UIObject"), picker);
        properties.insert(
            string(&objects[reference(pair, "UIKeyPath")]),
            reference(pair, "UIValue"),
        );
    }
    for (key, expected) in [
        ("datePickerMode", 0),
        ("minuteInterval", 5),
        ("preferredDatePickerStyle", 1),
    ] {
        assert_eq!(
            value(&objects[properties[key]], "NS.intval"),
            &Read::Int(expected)
        );
    }
    let locale = &objects[properties["locale"]];
    assert_eq!(locale.class, "NSLocale");
    assert_eq!(string(&objects[reference(locale, "NS.identifier")]), "ru");
    assert_eq!(
        value(&objects[properties["minimumDate"]], "NS.time"),
        &Read::Double(12345.0)
    );
    let action = class(&objects, "UIRuntimeEventConnection");
    assert_eq!(reference(action, "UISource"), picker);
    assert_eq!(
        objects[reference(action, "UIDestination")].class,
        "UIProxyObject"
    );
    assert_eq!(value(action, "UIEventMask"), &Read::Int(4096));
    assert_eq!(
        string(&objects[reference(action, "UILabel")]),
        "openValueChanged"
    );
    assert!(compile_xib(&body.replace("minuteInterval=\"5\"", "minuteInterval=\"7\"")).is_err());
    assert!(compile_xib(&body.replace("style=\"wheels\"", "style=\"bogus\"")).is_err());
}

#[test]
fn image_traits_keep_both_images_and_the_trait_condition() {
    let objects = compile_xib(
        r#"<imageView id="image" image="direction"><rect key="frame" width="40" height="40"/>
       <variation key="heightClass=regular-widthClass=regular" image="direction_big"/>
       </imageView>"#,
    )
    .unwrap();
    let storage = class(&objects, "_UIAttributeTraitStorage");
    assert_eq!(string(&objects[reference(storage, "UIKeyPath")]), "image");
    assert_eq!(objects[reference(storage, "UIObject")].class, "UIImageView");
    let records = &objects[reference(storage, "UIRecords")];
    let refs: Vec<_> = records
        .values
        .iter()
        .filter_map(|(_, v)| match v {
            Read::Object(i) => Some(*i),
            _ => None,
        })
        .collect();
    assert_eq!(refs.len(), 2);
    for (record, name) in refs.iter().zip(["direction_big", "direction"]) {
        let image = &objects[reference(&objects[*record], "UIValue")];
        assert_eq!(image.class, "UIImageNibPlaceholder");
        assert_eq!(string(&objects[reference(image, "UIResourceName")]), name);
    }
    let traits = &objects[reference(&objects[refs[0]], "UITraitCollection")];
    assert_eq!(
        value(
            traits,
            "UITraitCollectionBuiltinTrait-_UITraitNameHorizontalSizeClass"
        ),
        &Read::Int(2)
    );
    assert_eq!(
        value(
            traits,
            "UITraitCollectionBuiltinTrait-_UITraitNameVerticalSizeClass"
        ),
        &Read::Int(2)
    );
}

#[test]
fn placeholder_masks_exclude_only_design_time_constraints() {
    let body = r#"<view id="view"><rect key="frame" width="100" height="100"/>
      <constraints><constraint id="placeholder" firstAttribute="height" constant="44" placeholder="YES"/>
      <constraint id="runtime" firstAttribute="width" constant="100"/></constraints>
      <variation key="default"><mask key="constraints"><exclude reference="placeholder"/><exclude reference="runtime"/></mask></variation>
      <variation key="widthClass=regular"><mask key="constraints"><include reference="placeholder"/><include reference="runtime"/></mask></variation>
      </view>"#;
    let objects = compile_xib(body).unwrap();
    let constraints: Vec<_> = objects
        .iter()
        .filter(|o| o.class == "NSLayoutConstraint")
        .collect();
    assert_eq!(constraints.len(), 1);
    assert_eq!(value(constraints[0], "NSConstant"), &Read::Double(100.0));
    let storage = class(&objects, "_UIRelationshipTraitStorage");
    let candidates = &objects[reference(storage, "UIRelationshipCandidates")];
    assert_eq!(candidates.values.len(), 1);
    assert!(
        compile_xib(&body.replace("reference=\"placeholder\"", "reference=\"missing\""))
            .unwrap_err()
            .contains("unknown variation reference missing")
    );
}

#[test]
fn attributed_runs_use_foundation_varints_and_reuse_equal_dictionaries() {
    let body = format!(
        r#"<textView id="t"><rect key="frame" width="100" height="40"/>
        <attributedString key="attributedText">
        <fragment content="{}😀"><attributes><font key="NSFont" size="14" name="HelveticaNeue"/></attributes></fragment>
        <fragment content="B"><attributes><font key="NSFont" size="20" name="Georgia"/></attributes></fragment>
        <fragment content="C"><attributes><font key="NSFont" size="14" name="HelveticaNeue"/></attributes></fragment>
        </attributedString></textView>"#,
        "a".repeat(128)
    );
    let objects = compile_xib(&body).unwrap();
    let styled = class(&objects, "NSAttributedString");
    assert_eq!(
        value(&objects[reference(styled, "NSAttributeInfo")], "NS.bytes"),
        &Read::Data(vec![0x82, 1, 0, 1, 1, 1, 0])
    );
    let attrs = &objects[reference(styled, "NSAttributes")];
    assert_eq!(
        attrs
            .values
            .iter()
            .filter(|(key, _)| key == nib::ELEMENT)
            .count(),
        2
    );
    for bad in [
        body.replace("size=\"14\"", "size=\"NaN\""),
        body.replace("<attributes>", "<attributes unknown=\"YES\">"),
        body.replace("key=\"NSFont\"", "key=\"UnknownFont\""),
    ] {
        assert!(compile_xib(&bad).is_err());
    }
}
