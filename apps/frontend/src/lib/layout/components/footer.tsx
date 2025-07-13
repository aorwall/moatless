export const Footer = () => {
  return (
    <footer className="border-t py-4">
      <div className="container mx-auto px-6">
        <p className="text-xs text-muted-foreground text-center">
          Moatless Vibe - {new Date().getFullYear()}
        </p>
      </div>
    </footer>
  )
}
