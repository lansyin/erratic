#set page(
  height: 315pt,
  width: 720pt,
  margin: 0pt,
)
#set par(
  leading: 0.5em,
)
#set text(
  size: 1.125em,
)

#let code-targets(
  targets: (:), // label -> (line-no, match)
  preview: false,
  body,
) = {
  show raw.line: line => {
    if not line.body.has("children") {
      return line
    }
    let seq = line.body.children

    for (name, target) in targets {
      let target-line-no = target.at(0)
      let match = target.at(1)
      if target-line-no != line.number {
        continue
      }
      let el_index = 0
      let el_index_by_char = ()
      let prefix = ""
      let els = ()
      for el in seq {
        let segment
        if type(el) == dictionary {
          segment = "\0"
        } else {
          if el.has("child") {
            // styled([...])
            segment = el.child.text
          } else if el.has("text") {
            // [...]
            segment = el.text
          } else {
            // Unknown
            panic(el)
          }
        }
        assert(
          segment.codepoints().all(cp => cp <= "\u{7f}"),
          message: "code must contain only ASCII characters",
        )
        els = (..els, el)
        prefix += segment
        for _ in segment {
          el_index_by_char = (..el_index_by_char, el_index)
        }
        el_index += 1

        if not prefix.ends-with(match) {
          continue
        }

        let start_el_index = el_index_by_char.at(prefix.len() - match.len())
        if start_el_index == -1 {
          continue
        }

        let prev = els.slice(0, start_el_index)
        let rest = els.slice(start_el_index)
        els = (
          ..prev,
          (
            code-target: box[
              #box(stroke: if preview { green })[
                #for el in rest {
                  el
                }
              ] #label(name)
            ],
          ),
        )
        el_index_by_char = el_index_by_char.map(_ => -1)
        el_index = start_el_index + 1
      }
      seq = els
    }

    seq = seq.map(el => {
      if type(el) == dictionary {
        el.code-target
      } else {
        el
      }
    })

    if preview {
      seq = seq.map(el => box(stroke: rgb("#6200ff6e"))[#el])
    }

    seq.join()
  }

  body
}

#let wire(
  start,
  end,
  stroke: stroke(paint: rgb("#777777cc"), thickness: 0.5pt, dash: "dashed"),
  comment: none,
  comment_fill: rgb("777777cc"),
  comment_pos: 0.5,
) = {
  line(start: start, end: end, stroke: stroke)
  if comment != none {
    let delta_x = end.at(0) - start.at(0)
    let delta_y = end.at(1) - start.at(1)
    let pos_x = start.at(0) + delta_x * comment_pos
    let pos_y = start.at(1) + delta_y * comment_pos
    let comment_rendered = text(fill: comment_fill, size: .8em)[#comment]
    context {
      let comment_size = measure(comment_rendered)
      place(left + top, dx: pos_x - comment_size.width / 2, dy: pos_y - comment_size.height / 2, comment_rendered)
    }
  }
}

#let spline(points: (), stroke: stroke(paint: rgb("#777777cc"), thickness: 0.5pt, dash: "dashed"), tension: 0) = {
  assert(
    points.len() >= 2,
    message: "spline requires at least 2 points",
  )

  let sub(a, b) = (
    a.at(0) - b.at(0),
    a.at(1) - b.at(1),
  )

  let scale(a, s) = (
    a.at(0) * s,
    a.at(1) * s,
  )

  let components = (
    curve.move(points.at(0)),
  )

  for i in range(points.len() - 1) {
    let p0 = if i == 0 {
      points.at(0)
    } else {
      points.at(i - 1)
    }

    let p1 = points.at(i)
    let p2 = points.at(i + 1)

    let p3 = if i + 2 < points.len() {
      points.at(i + 2)
    } else {
      points.at(points.len() - 1)
    }

    // Catmull-Rom -> cubic Bezier
    //
    // C1 = P1 + (P2 - P0) / 6
    // C2 = P2 - (P3 - P1) / 6
    let base-c1 = p1 + scale(sub(p2, p0), tension / 6)
    let base-c2 = sub(
      p2,
      scale(sub(p3, p1), tension / 6),
    )

    // Special case: dip nearly-horizontal segments down so the line doesn't cover text.
    let near-horizontal = calc.abs(p1.at(1) - p2.at(1)) <= 1pt
    let dip-offset = if near-horizontal {
      0em
    } else {
      0em
    }

    let c1 = (base-c1.at(0), base-c1.at(1) + dip-offset)
    let c2 = (base-c2.at(0), base-c2.at(1) + dip-offset)

    components = (
      ..components,
      curve.cubic(
        c1,
        c2,
        p2,
      ),
    )
  }

  curve(
    stroke: stroke,
    ..components,
  )
}

#let repr_area(body, title: none, title-float: false) = [
  #if title != none and title-float {
    place(top + left, box[
      #place(bottom + left, dy: -.2em)[
        #text(size: .85em, style: "italic")[#title]
      ]
    ])
  }
  #stack(
    dir: ttb,
    spacing: .2em,
    ..if title != none and not title-float {
      (text(size: .85em, style: "italic")[#title],)
    },
    box(stroke: stroke(paint: rgb("#a0a0a066"), thickness: 0.5pt, dash: "dashed"), inset: .4em)[#body],
  )
]

#let repr_struct(name: "Struct", comment: none, ..fields) = [
  #let body = box(
    stroke: stroke(paint: rgb("#e0e0e0"), thickness: .5pt),
    inset: 0.4em,
    radius: 4pt,
    fill: rgb("#fafafad2"),
  )[
    #set align(top)
    #stack(
      dir: ltr,
      spacing: .2em,
      ..fields.pos(),
    )
  ]
  #context {
    let width = measure(body)
    box(width: width.width)[#grid(
      columns: (1fr, auto),
      rows: (auto, auto),
      gutter: .2em,
      text(size: .6em, font: "DejaVu Sans Mono")[#name],
      if comment != none {
        align(right, text(size: .7em)[#comment])
      },
      grid.cell(colspan: 2, body),
    )]
  }
]

#let repr_bytes(
  bytes,
  fill: rgb("#fdffdf"),
  stroke: stroke(paint: rgb("#25252566"), thickness: .5pt),
  head_label: none,
) = [
  #let byte_cell(byte) = box(fill: fill, inset: 2pt, stroke: stroke)[
    #text(font: "DejaVu Sans Mono", size: .7em)[#byte]
  ]
  #stack(
    dir: ltr,
    spacing: .15em,
    ..bytes
      .clusters()
      .slice(0, count: 1)
      .map(byte => [#byte_cell(byte) #if head_label != none {
          label(head_label)
        }]),
    ..bytes.clusters().slice(1).map(byte => byte_cell(byte)),
  )
]

#let repr_field(
  name,
  comment: none,
  fill: rgb("#faffdf"),
  stroke: stroke(paint: rgb("#25252566"), thickness: 0.25pt),
) = [
  #let cell = box(inset: 4pt, fill: fill, stroke: stroke)[
    #text[#name]
  ]
  #context {
    let cell_size = measure(cell)
    box[#stack(
      dir: ttb,
      spacing: .2em,
      cell,
      ..if comment != none {
        (box(width: cell_size.width)[#align(right, text(comment, size: 0.7em))],)
      },
    )]
  }
]

#let bottom_ends(name) = {
  let el = query(label(name)).first()
  let pos = el.location().position()
  let size = measure(el)
  let x = pos.x + size.width / 2
  let y = pos.y + size.height
  let half_width = size.width / 2 + .2em
  // let half_width = 0pt
  ((x - half_width, y), (x + half_width, y))
}

#let edge_center(name, align: top) = {
  let el = query(label(name)).first()
  let pos = el.location().position()
  let size = measure(el)
  let x = pos.x + size.width / 2
  let y = pos.y + size.height / 2
  let half_width = size.width / 2
  let half_height = size.height / 2
  if align.x == left { x -= half_width }
  if align.x == right { x += half_width }
  if align.y == top { y -= half_height }
  if align.y == bottom { y += half_height }
  (x, y)
}

// Contents

#place(right + top, dx: -20pt, dy: 20pt, block(
  stroke: rgb("#ffdaed"),
  fill: rgb("#ffe3f2"),
  inset: 1em,
)[#code-targets(
  targets: (
    interactive_login_line_1_ret: (1, "Result<()>"),
    interactive_login_line_4_try: (4, "?"),
    interactive_login_line_6_state: (6, "State"),
    interactive_login_line_6_continue: (6, "continue"),
  ),
  // preview: true,
)[
  ```rs
  fn interactive_login(&self) -> Result<()> {
      loop {
          let cred = cli::inquiry_credential();
          match self.login(cred).extract_state()? {
              Ok(_) => break
              Err((State::Unauthorized,_)) => continue,
  ```
]])

#place(center + horizon, dx: 0pt, dy: 0pt, block(
  stroke: rgb("#f2ddff"),
  fill: rgb("#f7ebff"),
  inset: 1em,
)[#code-targets(
  targets: (
    login_line_1_ret: (1, "Error<State>"),
    login_line_3_state: (3, "State"),
    login_line_3_try: (3, "?"),
    login_line_4_try: (4, "?"),
  ),
  // preview: true,
)[
  ```rs
  fn login(&self, cred: Cred) -> Result<(), Error<State>> {
      let resp = self.client.request(LOGIN_URL, cred)
          .with_state_if(State::Unauthorized, http::is_401)?;
      self.persist_apikey(resp.body)?;
      Ok(())
  }
  ```
]])

#place(left + bottom, dx: 20pt, dy: -20pt, block(
  stroke: rgb("#cff3f3"),
  fill: rgb("#e3ffff"),
  inset: 1em,
)[#code-targets(
  targets: (
    persist_apikey_line_1_ret: (1, "Result<()>"),
    persist_apikey_line_3_return: (3, "return"),
    persist_apikey_line_3_mkres: (3, "mkres!"),
    persist_apikey_line_3_end: (3, ";"),
    persist_apikey_line_5_try: (5, "?"),
  ),
  // preview: true,
)[
  ```rs
  fn persist_apikey(&self, key: String) -> Result<()> {
      let Some(conn) = self.db.get_conn() else {
          return mkres!("no database available");
      };
      Ok(conn.upsert_apikey(key)?)
  }
  ```
]])

#place(left + top, dx: 20pt, dy: 20pt, block(
  inset: .0em,
)[
  #repr_area(title: "rodata", title-float: true)[#stack(
    spacing: .5em,
    repr_struct(name: "str")[
      #repr_bytes("no database available", head_label: "str_head")
    ],
    [#repr_struct(
        name: "&'static str",
        [#repr_field("ptr") #label("static_str_ptr")],
        repr_field(
          "len",
        ),
      ) #label("static_str")],
  )]
  #repr_area(title: "stack")[
    #repr_struct(
      size: auto,
      name: "Error",
      comment: [#box(fill: rgb("#efffdd"))[tag]=00],
      [#repr_field("ptr", comment: "align=4") #label("error_tag00_ptr")],
      repr_field(
        "tag",
        comment: "2 bits",
        fill: rgb("#efffdd"),
      ),
    )
    #label("error_tag00")
  ]
])

#place(right + bottom, dx: -20pt, dy: -20pt, block[
  #set align(left)
  #stack(
    dir: rtl,
    spacing: .5em,
    stack(
      dir: btt,
      spacing: 1.5em,
      repr_area(title: "stack")[
        #stack(
          dir: btt,
          spacing: .5em,
          [
            #repr_struct(
              name: "Error",
              comment: [#box(fill: rgb("#efffdd"))[tag]=10],
              repr_field("state", comment: "< 1 usize"),
              repr_field(
                "pad",
                comment: "6 bits",
                fill: rgb("#ffffff00"),
              ),
              repr_field(
                "tag",
                comment: "2 bits",
                fill: rgb("#efffdd"),
              ),
            ) #label("error_tag10")
          ],
          [
            #repr_struct(
              name: "Error",
              comment: [#box(fill: rgb("#efffdd"))[tag]=01],
              [#repr_field("ptr", comment: "align=4") #label("error_tag01_ptr")],
              repr_field(
                "tag",
                comment: "2 bits",
                fill: rgb("#efffdd"),
              ),
            ) #label("error_tag01")
          ],
        )
      ],
      block[
        #box[#line(stroke: rgb("#ff7ddfdd"), length: 16pt)] #text(size: .9em, "No alloc.") \
        #box[#line(stroke: rgb("#74c0ffdd"), length: 16pt)] #text(size: .9em, "Alloc once.") \
        #box[#line(stroke: rgb("#ff9249dd"), length: 16pt)] #text(size: .9em, "Alloc on demand.") \
      ],
    ),
    repr_area(title: "heap")[
      #repr_struct(
        name: "BoxedBody",
        comment: "Unused fields collapse to ZST.",
        repr_field("vtable", comment: "align=4"),
        repr_field(
          "tag",
          comment: "2 bits",
          fill: rgb("#ddffef"),
        ),
        repr_field("state", comment: [#box(fill: rgb("#ddffef"))[tag]ged]),
        repr_field("error"),
        repr_field("context"),
      )
      #label("boxed_body")
    ],
  )
])

#place(right + bottom, dx: -20pt, dy: -20pt, block[
  #place(top + right, dy: .2em, block[
    #text("docs.rs/erratic")
  ])
])

#context {
  let interactive_login_line_1_ret = bottom_ends("interactive_login_line_1_ret")
  let interactive_login_line_4_try = bottom_ends("interactive_login_line_4_try")
  let interactive_login_line_6_state = bottom_ends("interactive_login_line_6_state")
  let interactive_login_line_6_continue = bottom_ends("interactive_login_line_6_continue")
  let login_line_1_ret = bottom_ends("login_line_1_ret")
  let login_line_3_try = bottom_ends("login_line_3_try")
  let login_line_3_state = bottom_ends("login_line_3_state")
  let login_line_4_try = bottom_ends("login_line_4_try")
  let persist_apikey_line_1_ret = bottom_ends("persist_apikey_line_1_ret")
  let persist_apikey_line_3_return = bottom_ends("persist_apikey_line_3_return")
  let persist_apikey_line_3_end = bottom_ends("persist_apikey_line_3_end")
  let persist_apikey_line_5_try = bottom_ends("persist_apikey_line_5_try")

  let persist_apikey_line_3_mkres_top = edge_center("persist_apikey_line_3_mkres", align: top)
  let error_tag00_bottom = edge_center("error_tag00", align: bottom)
  let error_tag00_ptr_top = edge_center("error_tag00_ptr", align: top)
  let static_str_bottom = edge_center("static_str", align: bottom)
  let static_str_ptr_top = edge_center("static_str_ptr", align: top)
  let str_head_bottom = edge_center("str_head", align: bottom)

  let persist_apikey_line_5_try_right = edge_center("persist_apikey_line_5_try", align: right)
  let error_tag01_left_top = edge_center("error_tag01", align: left + top)
  let error_tag01_ptr_left = edge_center("error_tag01_ptr", align: left)
  let boxed_body_right = edge_center("boxed_body", align: right)

  let login_line_3_try_right = edge_center("login_line_3_try", align: right)
  let error_tag10_left_top = edge_center("error_tag10", align: left + top)

  place(left + top, dx: 1pt, dy: 0.15em, spline(
    points: (
      persist_apikey_line_5_try,
      persist_apikey_line_1_ret,
      login_line_4_try,
      login_line_1_ret,
      interactive_login_line_4_try.rev(),
      interactive_login_line_1_ret.rev(),
    ).join(),
    stroke: rgb("#74c0ffdd"),
    // tension: 1,
  ))
  place(left + top, dx: 0pt, dy: 0.05em, spline(
    points: (
      persist_apikey_line_3_return,
      persist_apikey_line_3_end,
      persist_apikey_line_1_ret.rev(),
      login_line_4_try,
      login_line_1_ret,
      interactive_login_line_4_try.rev(),
      interactive_login_line_1_ret.rev(),
    ).join(),
    stroke: rgb("#ff7ddfdd"),
    // tension: 1,
  ))
  place(left + top, dx: 0pt, dy: 0.25em, spline(
    points: (
      login_line_3_state,
      login_line_3_try,
      login_line_1_ret.rev(),
      interactive_login_line_6_state,
      interactive_login_line_6_continue,
    ).join(),
    stroke: rgb("#ff9249dd"),
    // tension: 1,
  ))

  place(left + top, wire(
    persist_apikey_line_3_mkres_top,
    error_tag00_bottom,
  ))
  place(left + top, wire(
    error_tag00_ptr_top,
    static_str_bottom,
  ))
  place(left + top, wire(
    static_str_ptr_top,
    str_head_bottom,
  ))

  place(left + top, wire(
    persist_apikey_line_5_try_right,
    error_tag01_left_top,
  ))
  place(left + top, wire(
    error_tag01_ptr_left,
    boxed_body_right,
  ))

  place(left + top, wire(
    login_line_3_try_right,
    error_tag01_left_top,
    comment: [on #text(font: "DejaVu Sans Mono", size: .75em, "Result")],
    comment_pos: 0.65,
  ))
  place(left + top, wire(
    login_line_3_try_right,
    error_tag10_left_top,
    comment: [on #text(font: "DejaVu Sans Mono", size: .75em, "Option")],
    comment_pos: 0.7,
  ))
}
