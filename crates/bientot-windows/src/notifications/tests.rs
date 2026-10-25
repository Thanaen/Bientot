use super::*;

fn request(delivery_id: &str, title: &str, body: &str) -> NotificationRequest {
    NotificationRequest {
        delivery_id: delivery_id.to_owned(),
        title: title.to_owned(),
        body: body.to_owned(),
    }
}

#[test]
fn a_toast_has_the_title_and_the_body_and_carries_the_delivery_id() {
    assert_eq!(
        toast_xml(&request(
            "0f8fad5bd9cb469fa16570867728950e",
            "Title",
            "Body"
        )),
        concat!(
            r#"<toast launch="deliveryId=0f8fad5bd9cb469fa16570867728950e">"#,
            r#"<visual><binding template="ToastGeneric">"#,
            "<text>Title</text><text>Body</text>",
            "</binding></visual></toast>",
        )
    );
}

#[test]
fn text_is_escaped_for_xml() {
    assert_eq!(
        toast_xml(&request("id", "A & B", r#"<i>"it's"</i>"#)),
        concat!(
            r#"<toast launch="deliveryId=id"><visual><binding template="ToastGeneric">"#,
            "<text>A &amp; B</text>",
            "<text>&lt;i&gt;&quot;it&apos;s&quot;&lt;/i&gt;</text>",
            "</binding></visual></toast>",
        )
    );
}

#[test]
fn the_delivery_id_cannot_add_arguments() {
    assert!(
        toast_xml(&request(r#"a=b;c%d&"e"#, "Title", "Body"))
            .starts_with(r#"<toast launch="deliveryId=a%3Db%3Bc%25d&amp;&quot;e">"#)
    );
}

#[test]
fn a_click_hands_back_the_delivery_id_the_toast_carries() {
    for delivery_id in [
        "0f8fad5bd9cb469fa16570867728950e",
        "a=b;c%d",
        "%3B%253D%",
        "",
    ] {
        assert_eq!(
            super::delivery_id(&launch_arguments(delivery_id)).as_deref(),
            Some(delivery_id)
        );
    }
}

#[test]
fn the_delivery_id_is_read_among_other_arguments() {
    assert_eq!(
        super::delivery_id("action=open;deliveryId=a%3Db;other=x").as_deref(),
        Some("a=b")
    );
}

#[test]
fn a_click_without_a_delivery_id_has_none() {
    assert_eq!(super::delivery_id(""), None);
    assert_eq!(super::delivery_id("action=open"), None);
    assert_eq!(super::delivery_id("deliveryId"), None);
}
