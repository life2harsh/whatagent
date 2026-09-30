use std::sync::Arc;

use base64::{engine::general_purpose, Engine};
use chrono::{TimeZone, Utc};
use ring::signature::{KeyPair, RsaKeyPair, UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};
use wa_calendar_agent::{
    calendar::{
        validate::{normalize_recurrence, validate, Validation},
        CalendarOutcome, CalendarService, EventCandidate,
    },
    google::{
        auth::{AccessTokenSource, ServiceAccountAuth, StaticToken, CALENDAR_EVENTS_SCOPE},
        calendar::{event_id_for, GoogleCalendarClient},
        gemini::{ExtractedEvent, ExtractionInput, GeminiClient},
    },
};
use wiremock::{
    matchers::{body_string_contains, header, method, path, path_regex},
    Mock, MockServer, ResponseTemplate,
};

fn test_key() -> String {
    let output = std::process::Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
        ])
        .output()
        .expect("this test needs the openssl command to make a throwaway key");
    assert!(output.status.success(), "openssl genpkey failed");
    String::from_utf8(output.stdout).unwrap()
}

fn extracted(start: &str, end: &str) -> ExtractedEvent {
    ExtractedEvent {
        intent: "create_calendar_event".to_owned(),
        confidence: 0.97,
        title: Some(" OSDC meeting ".to_owned()),
        description: Some("".to_owned()),
        start: Some(start.to_owned()),
        end: Some(end.to_owned()),
        location: None,
        recurrence: None,
        needs_confirmation: false,
        ambiguity_reason: None,
        transcript: None,
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 11, 8, 0, 0).unwrap()
}

#[test]
fn confident_event_is_ready_and_trimmed() {
    let result = validate(
        &extracted("2026-09-12T18:00:00+05:30", "2026-09-12T19:00:00+05:30"),
        now(),
    );
    let Validation::Ready(candidate) = result else {
        panic!("expected ready, got {result:?}");
    };
    assert_eq!(candidate.title, "OSDC meeting");
    assert_eq!(candidate.description, None);
}

#[test]
fn low_confidence_with_valid_times_asks_for_confirmation() {
    let mut event = extracted("2026-09-12T18:00:00+05:30", "2026-09-12T19:00:00+05:30");
    event.confidence = 0.5;
    event.ambiguity_reason = Some("\"evening\" is vague".to_owned());
    assert!(matches!(
        validate(&event, now()),
        Validation::Confirm { reason: Some(reason), .. } if reason.contains("evening")
    ));
}

#[test]
fn invalid_requests_are_rejected() {
    let mut not_event = extracted("2026-09-12T18:00:00+05:30", "2026-09-12T19:00:00+05:30");
    not_event.intent = "not_a_calendar_request".to_owned();
    assert!(matches!(validate(&not_event, now()), Validation::Reject(_)));

    let no_offset = extracted("2026-09-12T18:00:00", "2026-09-12T19:00:00");
    assert!(matches!(validate(&no_offset, now()), Validation::Reject(_)));

    let backwards = extracted("2026-09-12T19:00:00+05:30", "2026-09-12T18:00:00+05:30");
    assert!(matches!(validate(&backwards, now()), Validation::Reject(_)));

    let past = extracted("2026-09-01T18:00:00+05:30", "2026-09-01T19:00:00+05:30");
    assert!(matches!(
        validate(&past, now()),
        Validation::Reject(message) if message.contains("past")
    ));

    let mut bad_rule = extracted("2026-09-12T18:00:00+05:30", "2026-09-12T19:00:00+05:30");
    bad_rule.recurrence = Some("FREQ=HOURLY".to_owned());
    assert!(matches!(validate(&bad_rule, now()), Validation::Reject(_)));
}

#[test]
fn recurrence_rules_are_constrained_and_normalized() {
    assert_eq!(
        normalize_recurrence("rrule:freq=weekly;byday=we"),
        Some("FREQ=WEEKLY;BYDAY=WE".to_owned())
    );
    assert_eq!(
        normalize_recurrence("FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=6"),
        Some("FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=6".to_owned())
    );
    assert!(normalize_recurrence("FREQ=DAILY;UNTIL=20261231T000000Z").is_some());
    assert!(normalize_recurrence("FREQ=DAILY;UNTIL=2026-12-31").is_none());
    assert!(normalize_recurrence("FREQ=WEEKLY;BYDAY=XX").is_none());
    assert!(normalize_recurrence("FREQ=WEEKLY;BYSETPOS=1").is_none());
    assert!(normalize_recurrence("BYDAY=MO;FREQ=WEEKLY").is_none());
    assert!(normalize_recurrence("FREQ=DAILY;INTERVAL=0").is_none());
}

#[test]
fn event_ids_use_only_base32hex_characters() {
    let id = event_id_for("3EB013E67F8D09BFD931E0");
    assert!(id.len() >= 5);
    assert!(id
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c)));
}

#[tokio::test]
async fn service_account_signs_a_verifiable_jwt_and_caches_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ya29.test",
            "expires_in": 3599,
            "token_type": "Bearer"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let test_key = test_key();
    let key_json = serde_json::json!({
        "client_email": "agent@project.iam.gserviceaccount.com",
        "private_key": test_key,
        "token_uri": format!("{}/token", server.uri()),
    })
    .to_string();
    let auth =
        ServiceAccountAuth::from_json(reqwest::Client::new(), &key_json, CALENDAR_EVENTS_SCOPE)
            .unwrap();

    assert_eq!(auth.access_token().await.unwrap(), "ya29.test");
    assert_eq!(auth.access_token().await.unwrap(), "ya29.test");

    let request = &server.received_requests().await.unwrap()[0];
    let form: Vec<(String, String)> = url::form_urlencoded::parse(&request.body)
        .into_owned()
        .collect();
    let assertion = &form.iter().find(|(key, _)| key == "assertion").unwrap().1;
    let (signing_input, signature) = assertion.rsplit_once('.').unwrap();
    let claims_b64 = signing_input.split('.').nth(1).unwrap();
    let claims: serde_json::Value =
        serde_json::from_slice(&general_purpose::URL_SAFE_NO_PAD.decode(claims_b64).unwrap())
            .unwrap();
    assert_eq!(claims["iss"], "agent@project.iam.gserviceaccount.com");
    assert_eq!(claims["scope"], CALENDAR_EVENTS_SCOPE);

    let der: String = test_key
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let key_pair = RsaKeyPair::from_pkcs8(&general_purpose::STANDARD.decode(der).unwrap()).unwrap();
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key_pair.public_key().as_ref())
        .verify(
            signing_input.as_bytes(),
            &general_purpose::URL_SAFE_NO_PAD.decode(signature).unwrap(),
        )
        .expect("JWT signature verifies");
}

fn gemini_reply(event: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "candidates": [{ "content": { "parts": [
            { "text": "thinking...", "thought": true },
            { "text": event.to_string() }
        ] } }]
    }))
}

fn service(server: &MockServer) -> CalendarService {
    let http = reqwest::Client::new();
    let gemini = GeminiClient::new(
        http.clone(),
        url::Url::parse(&server.uri()).unwrap(),
        "gemini-key".to_owned(),
        "text-model".to_owned(),
        "audio-model".to_owned(),
    );
    let calendar = GoogleCalendarClient::new(
        http,
        url::Url::parse(&format!("{}/calendar/v3", server.uri())).unwrap(),
        "me@example.com".to_owned(),
        Arc::new(StaticToken("calendar-token".to_owned())),
    );
    CalendarService::new(gemini, calendar, chrono_tz::Asia::Kolkata, 60)
}

fn far_future_event() -> serde_json::Value {
    serde_json::json!({
        "intent": "create_calendar_event",
        "confidence": 0.98,
        "title": "Solve NPTEL assignment",
        "description": null,
        "start": "2099-09-16T15:00:00+05:30",
        "end": "2099-09-16T16:00:00+05:30",
        "location": null,
        "recurrence": "FREQ=WEEKLY;BYDAY=WE",
        "needs_confirmation": false,
        "ambiguity_reason": null,
        "transcript": null
    })
}

#[tokio::test]
async fn text_request_creates_a_recurring_event_with_a_deterministic_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/text-model:generateContent"))
        .and(header("x-goog-api-key", "gemini-key"))
        .and(body_string_contains("every Wednesday at 3pm"))
        .and(body_string_contains("Asia/Kolkata"))
        .respond_with(gemini_reply(far_future_event()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(
            r"^/calendar/v3/calendars/me(@|%40)example\.com/events$",
        ))
        .and(header("authorization", "Bearer calendar-token"))
        .and(body_string_contains(event_id_for("message-1")))
        .and(body_string_contains("RRULE:FREQ=WEEKLY;BYDAY=WE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": event_id_for("message-1"),
            "summary": "Solve NPTEL assignment",
            "start": { "dateTime": "2099-09-16T15:00:00+05:30" },
            "end": { "dateTime": "2099-09-16T16:00:00+05:30" },
            "htmlLink": "https://calendar.google.com/event?eid=x"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let outcome = service(&server)
        .handle(
            "message-1",
            ExtractionInput::Text("cal every Wednesday at 3pm solve NPTEL assignment".to_owned()),
        )
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        CalendarOutcome::Created { message, .. } if message.contains("Solve NPTEL assignment")
    ));
}

#[tokio::test]
async fn replayed_event_reports_already_added() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/events$"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;
    let candidate = EventCandidate {
        title: "Standup".to_owned(),
        start: chrono::DateTime::parse_from_rfc3339("2099-09-16T10:00:00+05:30").unwrap(),
        end: chrono::DateTime::parse_from_rfc3339("2099-09-16T10:15:00+05:30").unwrap(),
        description: None,
        location: None,
        recurrence: None,
    };

    let outcome = service(&server)
        .create("message-1", &candidate)
        .await
        .unwrap();

    assert!(matches!(outcome, CalendarOutcome::AlreadyAdded { .. }));
}

#[tokio::test]
async fn ambiguous_voice_note_needs_confirmation_without_creating_an_event() {
    let server = MockServer::start().await;
    let mut event = far_future_event();
    event["confidence"] = serde_json::json!(0.6);
    event["needs_confirmation"] = serde_json::json!(true);
    event["transcript"] = serde_json::json!("NPTEL on Wednesday evening");
    Mock::given(method("POST"))
        .and(path("/v1beta/models/audio-model:generateContent"))
        .and(body_string_contains("inlineData"))
        .and(body_string_contains("audio/ogg; codecs=opus"))
        .respond_with(gemini_reply(event))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"/events$"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let outcome = service(&server)
        .handle(
            "voice-1",
            ExtractionInput::Audio {
                mime_type: Some("audio/ogg; codecs=opus".to_owned()),
                data: b"voice-note".to_vec(),
            },
        )
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        CalendarOutcome::NeedsConfirmation { request_id, message, .. }
            if request_id == "voice-1" && message.contains("Heard: \"NPTEL on Wednesday evening\"")
    ));
}

#[tokio::test]
async fn upstream_errors_carry_the_google_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"generateContent$"))
        .respond_with(ResponseTemplate::new(429).set_body_string("RESOURCE_EXHAUSTED"))
        .mount(&server)
        .await;

    let error = service(&server)
        .handle(
            "message-1",
            ExtractionInput::Text("cal lunch tomorrow".to_owned()),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("Gemini"));
    assert!(error.to_string().contains("RESOURCE_EXHAUSTED"));
}
