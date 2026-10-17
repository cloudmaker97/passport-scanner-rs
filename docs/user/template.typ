#import "handbook.typ": *
#show: handbook.with(
  title: [$title$],
  subtitle: [$subtitle$],
  author: [$for(author)$$author$$sep$, $endfor$],
  version: $if(version)$"$version$"$else$none$endif$,
  lang: "$lang$",
)

$body$
