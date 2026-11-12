use halcyon_core::imap::parse_header_block;

#[test]
fn marketplace_conversation_one_shares_reply_to_and_thread() {
    let m1 = parse_header_block(include_bytes!(
        "fixtures/headers/01-marketplace-conv1-msg1.eml"
    ));
    let m2 = parse_header_block(include_bytes!(
        "fixtures/headers/02-marketplace-conv1-msg2.eml"
    ));
    let m3 = parse_header_block(include_bytes!(
        "fixtures/headers/03-marketplace-conv1-msg3.eml"
    ));

    assert_eq!(m1.from_name.as_deref(), Some("Kleinanzeigen-Nachrichten"));
    assert_eq!(m1.from_addr.as_deref(), Some("noreply@kleinmarkt.example"));
    assert_eq!(
        m1.reply_to.as_deref(),
        Some("conv-1111@reply.kleinmarkt.example")
    );
    assert_eq!(
        m1.subject.as_deref(),
        Some("Neue Nachricht zu: Vintage Fahrrad 28 Zoll")
    );
    assert_eq!(
        m1.message_id.as_deref(),
        Some("mk-1111-1@kleinmarkt.example")
    );
    assert!(m1.date.is_some());

    assert_eq!(m2.reply_to, m1.reply_to);
    assert_eq!(m2.subject.as_deref(), Some("Re: Vintage Fahrrad 28 Zoll"));
    assert_eq!(
        m2.in_reply_to.as_deref(),
        Some("mk-1111-1@kleinmarkt.example")
    );
    assert_eq!(m2.references, vec!["mk-1111-1@kleinmarkt.example"]);

    assert_eq!(m3.reply_to, m1.reply_to);
    assert_eq!(
        m3.in_reply_to.as_deref(),
        Some("mk-1111-2@kleinmarkt.example")
    );
    assert_eq!(
        m3.references,
        vec![
            "mk-1111-1@kleinmarkt.example",
            "mk-1111-2@kleinmarkt.example"
        ]
    );
}

#[test]
fn marketplace_conversation_two_has_distinct_reply_to() {
    let m1 = parse_header_block(include_bytes!(
        "fixtures/headers/01-marketplace-conv1-msg1.eml"
    ));
    let other = parse_header_block(include_bytes!(
        "fixtures/headers/04-marketplace-conv2-msg1.eml"
    ));

    assert_eq!(
        other.reply_to.as_deref(),
        Some("conv-2222@reply.kleinmarkt.example")
    );
    assert_ne!(other.reply_to, m1.reply_to);
    assert_eq!(
        other.subject.as_deref(),
        Some("Neue Nachricht zu: Schreibtisch Eiche massiv")
    );
}

#[test]
fn invoices_and_payment_reminder_parse_cleanly() {
    let tel = parse_header_block(include_bytes!("fixtures/headers/05-invoice-telecom.eml"));
    assert_eq!(tel.from_addr.as_deref(), Some("rechnung@telekom.example"));
    assert_eq!(tel.to, vec!["max.muster@mail.example"]);
    assert_eq!(
        tel.subject.as_deref(),
        Some("Ihre Rechnung November 2026 - Kundennummer 90012345")
    );
    assert!(tel.list_unsubscribe.is_empty());
    assert!(!tel.list_unsubscribe_post);

    let pwr = parse_header_block(include_bytes!("fixtures/headers/06-invoice-power.eml"));
    assert_eq!(
        pwr.from_addr.as_deref(),
        Some("abrechnung@stromwerk.example")
    );
    assert_eq!(pwr.to, vec!["Erika Beispiel <erika.beispiel@post.example>"]);
    assert_eq!(
        pwr.subject.as_deref(),
        Some("Ihre Stromabrechnung 2026 - Vertrag 445566")
    );

    let shop = parse_header_block(include_bytes!("fixtures/headers/07-invoice-shop.eml"));
    assert_eq!(shop.from_addr.as_deref(), Some("buchhaltung@shop.example"));
    assert_eq!(
        shop.subject.as_deref(),
        Some("Rechnung zu Ihrer Bestellung #ORD-2026-8841")
    );

    let rem = parse_header_block(include_bytes!("fixtures/headers/08-payment-reminder.eml"));
    assert_eq!(rem.from_addr.as_deref(), Some("mahnung@stromwerk.example"));
    assert_eq!(
        rem.subject.as_deref(),
        Some("Zahlungserinnerung / 1. Mahnung zu Rechnung RE-2026-102")
    );
}

#[test]
fn order_shipping_delivery_sequence() {
    let ord = parse_header_block(include_bytes!("fixtures/headers/09-order-confirmation.eml"));
    let ship = parse_header_block(include_bytes!(
        "fixtures/headers/10-shipping-confirmation.eml"
    ));
    let deliv = parse_header_block(include_bytes!(
        "fixtures/headers/11-delivery-notification.eml"
    ));

    assert_eq!(ord.from_addr.as_deref(), Some("bestellung@shop.example"));
    assert_eq!(
        ord.subject.as_deref(),
        Some("Bestellbestaetigung #ORD-2026-7710")
    );

    assert_eq!(ship.from_addr.as_deref(), Some("versand@shop.example"));
    assert_eq!(
        ship.subject.as_deref(),
        Some("Ihre Bestellung #ORD-2026-7710 wurde versandt")
    );

    assert_eq!(
        deliv.from_addr.as_deref(),
        Some("status@paketdienst.example")
    );
    assert_eq!(
        deliv.subject.as_deref(),
        Some("Zugestellt: Sendung zu Bestellung #ORD-2026-7710")
    );
    assert!(ord.date < ship.date && ship.date < deliv.date);
}

#[test]
fn otp_and_login_alerts() {
    let de = parse_header_block(include_bytes!("fixtures/headers/12-otp-german.eml"));
    assert_eq!(de.from_addr.as_deref(), Some("no-reply@bank.example"));
    assert_eq!(de.subject.as_deref(), Some("Ihr Einmalcode: 482913"));

    let en = parse_header_block(include_bytes!("fixtures/headers/13-otp-english.eml"));
    assert_eq!(en.from_addr.as_deref(), Some("security@portal.example"));
    assert_eq!(
        en.subject.as_deref(),
        Some("Your verification code for Portal.example")
    );

    let alert = parse_header_block(include_bytes!("fixtures/headers/14-login-alert.eml"));
    assert_eq!(alert.from_addr.as_deref(), Some("alert@portal.example"));
    assert_eq!(
        alert.subject.as_deref(),
        Some("Neue Anmeldung bei Ihrem Konto")
    );
}

#[test]
fn newsletters_cover_one_click_mailto_and_missing_unsubscribe() {
    let one_click = parse_header_block(include_bytes!(
        "fixtures/headers/15-newsletter-oneclick.eml"
    ));
    assert_eq!(one_click.list_id.as_deref(), Some("technik.news.example"));
    assert_eq!(
        one_click.list_unsubscribe,
        vec![
            "https://news.example/unsub/abc",
            "mailto:unsub@news.example?subject=stop",
        ]
    );
    assert!(one_click.list_unsubscribe_post);

    let mailto_only = parse_header_block(include_bytes!(
        "fixtures/headers/16-newsletter-mailto-only.eml"
    ));
    assert_eq!(
        mailto_only.list_id.as_deref(),
        Some("rundbrief.verein.example")
    );
    assert_eq!(
        mailto_only.list_unsubscribe,
        vec!["mailto:abmelden@verein.example?subject=unsubscribe"]
    );
    assert!(!mailto_only.list_unsubscribe_post);

    let none = parse_header_block(include_bytes!(
        "fixtures/headers/17-newsletter-no-unsub.eml"
    ));
    assert!(none.list_id.is_none());
    assert!(none.list_unsubscribe.is_empty());
    assert!(!none.list_unsubscribe_post);
}

#[test]
fn reply_chain_preserves_in_reply_to_and_references() {
    let root = parse_header_block(include_bytes!("fixtures/headers/18-thread-root.eml"));
    let reply = parse_header_block(include_bytes!("fixtures/headers/19-thread-reply.eml"));
    let nested = parse_header_block(include_bytes!(
        "fixtures/headers/20-thread-nested-reply.eml"
    ));

    assert_eq!(root.message_id.as_deref(), Some("thread-1@team.example"));
    assert!(root.in_reply_to.is_none());
    assert!(root.references.is_empty());

    assert_eq!(reply.in_reply_to.as_deref(), Some("thread-1@team.example"));
    assert_eq!(reply.references, vec!["thread-1@team.example"]);

    assert_eq!(nested.in_reply_to.as_deref(), Some("thread-2@team.example"));
    assert_eq!(
        nested.references,
        vec!["thread-1@team.example", "thread-2@team.example"]
    );
    assert_eq!(nested.cc, vec!["Anna Admin <anna@team.example>"]);
}

#[test]
fn rfc2047_encoded_headers_and_folded_lines() {
    let b64 = parse_header_block(include_bytes!(
        "fixtures/headers/21-encoding-utf8-base64.eml"
    ));
    assert_eq!(b64.from_name.as_deref(), Some("Jürgen Müller"));
    assert_eq!(b64.from_addr.as_deref(), Some("juergen@mail.example"));
    assert_eq!(b64.subject.as_deref(), Some("Rechnung für März 2026 🎉"));

    let qp_utf8 = parse_header_block(include_bytes!("fixtures/headers/22-encoding-utf8-qp.eml"));
    assert_eq!(qp_utf8.from_name.as_deref(), Some("Sabine Schön"));
    assert_eq!(
        qp_utf8.subject.as_deref(),
        Some("Prüfung der Bücher € 12,50")
    );

    let qp_iso = parse_header_block(include_bytes!(
        "fixtures/headers/23-encoding-iso8859-1-qp.eml"
    ));
    assert_eq!(qp_iso.from_name.as_deref(), Some("Jürgen Müller"));
    assert_eq!(qp_iso.subject.as_deref(), Some("Bestätigung für Max"));

    let folded = parse_header_block(include_bytes!("fixtures/headers/24-folded-headers.eml"));
    assert_eq!(
        folded.subject.as_deref(),
        Some("Abstimmung zum langen Projektplan fuer das kommende Quartal")
    );
    assert_eq!(
        folded.to,
        vec![
            "Max Muster <max.muster@mail.example>",
            "Erika Beispiel <erika.beispiel@post.example>",
        ]
    );
    assert_eq!(
        folded.references,
        vec!["ref-1@team.example", "ref-2@team.example"]
    );
    assert_eq!(
        folded.list_unsubscribe,
        vec!["https://team.example/unsub", "mailto:unsub@team.example",]
    );
}

#[test]
fn group_addresses_and_multiple_recipients() {
    let group = parse_header_block(include_bytes!("fixtures/headers/25-group-address.eml"));
    assert_eq!(group.to, vec!["anna@team.example", "bernd@team.example"]);

    let multi = parse_header_block(include_bytes!("fixtures/headers/26-multi-to-and-cc.eml"));
    assert_eq!(
        multi.to,
        vec![
            "Max Muster <max.muster@mail.example>",
            "erika.beispiel@post.example",
        ]
    );
    assert_eq!(
        multi.cc,
        vec!["Anna Admin <anna@team.example>", "bernd@team.example",]
    );
}

#[test]
fn broken_headers_do_not_panic_and_return_sane_defaults() {
    let missing_date = parse_header_block(include_bytes!(
        "fixtures/headers/27-broken-missing-date.eml"
    ));
    assert_eq!(missing_date.date, None);
    assert_eq!(
        missing_date.subject.as_deref(),
        Some("Nachricht ohne Datum")
    );

    let invalid_date = parse_header_block(include_bytes!(
        "fixtures/headers/28-broken-invalid-date.eml"
    ));
    assert_eq!(invalid_date.date, None);
    assert_eq!(
        invalid_date.subject.as_deref(),
        Some("Nachricht mit kaputtem Datum")
    );

    let no_addr = parse_header_block(include_bytes!(
        "fixtures/headers/29-broken-from-no-addr.eml"
    ));
    assert_eq!(no_addr.from_addr, None);
    assert_eq!(
        no_addr.subject.as_deref(),
        Some("Absender ohne Mailadresse")
    );

    let dup = parse_header_block(include_bytes!(
        "fixtures/headers/30-broken-duplicate-headers.eml"
    ));
    assert!(dup.from_addr.is_some());
    assert!(dup.subject.is_some());

    let long_subj = parse_header_block(include_bytes!(
        "fixtures/headers/31-broken-very-long-subject.eml"
    ));
    let subj = long_subj.subject.expect("long subject should parse");
    assert!(subj.len() > 300);
    assert!(subj.starts_with("Sehr langer Betreff Teil A "));
}
